# Importing Personal Memory

This guide walks through seeding a Boswell instance with facts about yourself —
for example, by asking an assistant you already use (Claude, ChatGPT) to export
what it knows about you, then loading that into Boswell as claims.

Boswell stores everything as confidence-weighted **claims** in the form
`(subject, predicate, object)`, not flat facts. The loadable format is a JSON
array (see [Claim JSON schema](#claim-json-schema)), consumed by
`boswell learn`. `boswell validate` checks a file offline before you load it.

## 1. Export from an assistant

Paste the prompt below into Claude and/or ChatGPT. It instructs the assistant to
emit claims in exactly the schema `boswell learn` expects. Save each reply to its
own file (e.g. `claude-memories.json`, `chatgpt-memories.json`).

> Swap `person:jd` for whatever self-id you like — just keep it identical across
> both exports so the two sources line up on the same subject.

````text
You are helping me export what you know about me into a personal knowledge base
called Boswell. Boswell stores everything as confidence-weighted CLAIMS in the
form (subject, predicate, object) — not flat facts.

Based on everything you know about me — from our past conversations and any saved
memory or context you have — produce a comprehensive list of claims about me as a
single JSON array. Output ONLY the JSON, inside one ```json code block, with no
commentary before or after.

Each element must have exactly this shape:

{
  "subject":   "namespace:value",
  "predicate": "namespace:value",
  "object":    "namespace:value",
  "confidence": { "lower": 0.0, "upper": 1.0 },
  "tier": "permanent" | "project" | "task"
}

ENTITY STRINGS (subject / predicate / object):
- Always "namespace:value", exactly one colon.
- Lowercase; join words with hyphens; no spaces, no extra colons, no quotes.
- Use ME as the subject for facts about me. My canonical id is `person:jd` — use
  it for every fact that is about me. Use a different subject only when the fact
  is genuinely about another entity (e.g. `project:boswell`, `place:portland`).
- Predicates: `rel:` for relationships (`rel:works-as`, `rel:lives-in`,
  `rel:uses`, `rel:knows`, `rel:enjoys`), `attr:` for properties
  (`attr:age`, `attr:communication-style`).
- Objects are the value: `role:software-engineer`, `lang:rust`,
  `place:portland-oregon`, `value:directness`, `num:45`.

CONFIDENCE — set it honestly per claim (lower/upper form an interval; widen it
when less sure):
- 0.9–1.0: I stated it explicitly and it's unlikely to change.
- 0.7–0.9: strongly implied or consistent across our conversations.
- 0.4–0.7: a reasonable inference you're less certain about.

TIER — how durable the claim is:
- "permanent": core identity that essentially never changes (birthplace, native
  language, deeply held values).
- "project": stable but could change over months/years (job, city, current
  tools, ongoing projects, preferences).
- "task": short-term / current-moment facts (should be rare here).

RULES:
- One fact per claim. Split compound facts into separate claims.
- Prefer many small, precise claims over a few vague ones.
- Do NOT invent facts. If unsure, lower the confidence or omit it.
- Cover real breadth: identity, background, work, skills, tools, preferences,
  communication style, goals, relationships, and anything else you genuinely know.

EXAMPLE of the exact format:
```json
[
  { "subject": "person:jd", "predicate": "rel:works-as", "object": "role:software-engineer", "confidence": { "lower": 0.9, "upper": 1.0 }, "tier": "project" },
  { "subject": "person:jd", "predicate": "rel:uses", "object": "lang:rust", "confidence": { "lower": 0.8, "upper": 0.95 }, "tier": "project" },
  { "subject": "person:jd", "predicate": "attr:communication-style", "object": "value:direct-and-concise", "confidence": { "lower": 0.7, "upper": 0.9 }, "tier": "project" }
]
```

Now produce my list.
````

## 2. Validate before loading

Assistants occasionally leak prose or emit a trailing comma. Check each file
offline first — this needs no running server:

```bash
boswell validate claude-memories.json
boswell validate chatgpt-memories.json
```

It reports every problem per claim (bad entity format, out-of-range or inverted
confidence, unknown tier, malformed JSON) and exits non-zero if anything is
wrong, so you can gate a load in a script:

```bash
boswell validate memories.json && boswell learn memories.json
```

## 3. Load into a running instance

```bash
# one-time prerequisites
brew install protobuf          # or: apt-get install protobuf-compiler
ollama pull embeddinggemma     # the embedder Boswell uses for semantic search

cargo build --release

# terminal 1 — the instance server (store + embedder)
./target/release/boswell-server --config config/instance.toml

# terminal 2 — the router (session/routing layer the CLI talks to)
./target/release/boswell-router --config config/router.toml

# terminal 3 — load (each CLI command auto-connects via the router)
./target/release/boswell learn claude-memories.json
./target/release/boswell learn chatgpt-memories.json
```

## 4. Query it

```bash
boswell query  --subject person:jd
boswell search "what do I value and how do I like to work"
```

## Claim JSON schema

`learn` / `validate` accept a JSON **array** of objects:

| Field        | Required | Notes                                                                 |
|--------------|----------|-----------------------------------------------------------------------|
| `subject`    | yes      | `namespace:value`, exactly one colon, non-empty parts                 |
| `predicate`  | yes      | `namespace:value`                                                     |
| `object`     | yes      | `namespace:value`                                                     |
| `confidence` | no       | `{ "lower": f64, "upper": f64 }`, each in `[0,1]`, `lower ≤ upper`; defaults to `{0.5, 1.0}` |
| `tier`       | no       | one of `ephemeral`, `task`, `project`, `permanent`; defaults to the command's `--tier` (`task`) |

The claim's namespace is taken from the **subject's** namespace, so grouping
subjects by namespace (`person:…`, `project:…`, `place:…`) organizes the store
for later namespace-scoped queries and maintenance.

## Notes

- **Overlap across sources.** Loading both a Claude and a ChatGPT export creates
  **separate** claims for overlapping facts — loading does not deduplicate by
  content. Treat this as two sources corroborating the same thing, which is what
  Boswell's confidence model is built to reconcile. To avoid it, merge and
  hand-dedupe the files first.
- **Maintenance is off by default.** The Janitor, Synthesizer, and Contradiction
  services are disabled in `config/instance.toml`, so nothing will decay or GC
  your seed data on load. Turn them on later (see the `[janitor]`,
  `[synthesizer]`, and `[contradiction]` sections) once you want the memory to
  behave organically — and note that `permanent`-tier claims never decay, while
  `project`/`task` claims fade over time without reinforcement (ADR-007).
