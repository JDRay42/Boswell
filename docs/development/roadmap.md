# Boswell Roadmap

**This file is the single source of truth for where Boswell stands.** If you want to know
what is built, what is being built, and what is deliberately not being built, this is the
only place to look. Design lives in [`docs/architecture/`](../architecture/) and decisions
in [`docs/ADRs/`](../ADRs/); neither carries status.

## How to read it

Work is grouped into **workstreams** — long-lived areas of the system — each holding
**slices**, the unit that actually ships. A slice's **title is its identity**; nothing is
numbered, because numbering implies an order the work does not have. Procedures, devAuth
and the gateway all arrived mid-flight and none of them fitted the phase plan that existed
at the time, which is how that plan came to describe a system nobody was building. New
workstreams get appended. New slices get inserted wherever they belong.

PR numbers on a shipped slice are **citations, not indices** — they point at the diff and
the reasoning, which are the two things that never drift.

Every slice carries one of four statuses, defined in [`CONTEXT.md`](../../CONTEXT.md):

- **shipped** — on `main`, with the PRs that did it
- **in flight** — someone is working on it now
- **open** — not done, no decision made
- **deferred** — not done *on purpose*, with the reason and what would unblock it

The distinction between *open* and *deferred* is the one worth protecting. Procedure
learning is not waiting for someone to get to it; it is waiting for a corpus of real
episodes to exist, and starting it early would produce a worse extractor. That reasoning is
the thing that evaporates between sessions.

## Keeping it true

**This file is updated in the same PR as the work it describes.** A feature PR that leaves
the roadmap untouched is incomplete. This rule exists because the previous arrangement —
status recorded separately, after the fact, by whoever remembered — produced a roadmap with
zero of thirty-six boxes checked while two phases were essentially complete. Status that
lives outside the change does not survive contact with a real week.

---

## Claims core

The declarative substrate: what is so. Domain model, storage, extraction, validation.

- **Domain model — claims, confidence intervals, tiers, namespaces.** `boswell-domain`,
  no external dependencies but `uuid`, property-tested with proptest.
  *shipped* (#1 standardised identifiers on UUIDv7, superseding ADR-011)
- **SQLite claim store with an HNSW vector index.** The one storage adapter that ships.
  *shipped*
- **Embeddings persist across restart.** The index is in-memory and replayed at startup;
  missing embeddings are backfilled. ADR-014 chose this over an on-disk index file.
  *shipped* (#19)
- **Semantic search returns results by default; assert stops collapsing confidence
  intervals.** *shipped* (#20)
- **`schema_info` seed insert is idempotent**, so a file-backed database can reopen.
  *shipped* (#11)
- **Extractor — unstructured text to structured claims.** Chunking, prompting, parsing,
  provenance linkage back to the source. *shipped*
- **Gatekeeper validation.** Tier/confidence rules, duplicate detection, semantic
  near-duplicate rejection against the store. *shipped*
- **`boswell validate` and the personal-memory import path.** *shipped* (#3, and #30 for
  the file-argument pairing)
- **Tier validation on the gRPC assert path.** The tier/confidence floor is now enforced
  on `Assert` and on `Learn`, which is the same hole in batch form — before this, a caller
  could write a `permanent` claim it was 10% sure of, because the Gatekeeper was only ever
  reached through the Extractor. `boswell-grpc` depends on `boswell-gatekeeper` and calls
  its rule rather than restating it. Only the tier rule is applied: the duplicate rules
  stay behind `Gatekeeper::validate`, because the store's assert path treats a repeat as
  corroboration rather than as an error, and entity-format validation would reject the
  bare subjects the wire has always accepted. The floor is configurable off via
  `BosWellServiceImpl::with_validation_config`, since an operator with a different
  confidence convention otherwise could not run the server.
  *shipped* (#48)
- **LLM-backed semantic validation in the Gatekeeper.** The crate has no LLM dependency
  and the validator makes no LLM call. Distinct from the semantic *duplicate* detection,
  which ships.
  *open*

## Procedural memory

The other half of what Boswell remembers: how to do things. Design of record is
[`15-procedural-memory.md`](../architecture/15-procedural-memory.md), which holds the
reasoning; this section holds the state.

- **Design doc — procedural and goal memory for agent teams.** *shipped* (#8)
- **`Procedure` entity, store, effectiveness and the reporting model.** *shipped* (#10)
- **`Goal` entity, `expand` traversal, decision aids.** *shipped* (#12)
- **Provenance write path, promotion Gatekeeper, live Janitor sweep.** *shipped* (#13)
- **Effectiveness capture via execution receipts — "silence is not success."** An
  unanswered receipt expires as `unknown` and counts against the procedure. *shipped* (#17)
- **One name for each thing.** The store *issues* a procedure; the obligation is an
  *execution receipt*; the executor answers with an *outcome report*.
  *shipped* (#22, #23; glossary in #32)
- **Graph integrity under decay — `CascadeAndOrphan`.** Both candidate rules from §8 #5
  were prototyped and neither was adopted; the reasoning is §8.2. *shipped* (#28)
- **Cycle guards, both halves** — rejected on write, and a `DescentGuard` at traversal.
  *shipped* (#29)
- **Semantic intent match for procedure and goal retrieval.** Retrieval currently falls
  back to case-insensitive substring matching. Marked in place at
  `boswell-store/src/procedure_store.rs:295`, `goal_store.rs:114`, and `schema.sql:185`.
  *open*
- **Push the triple match into SQL** rather than filtering in Rust. `ClaimQuery` grew
  exact `subject`/`predicate`/`object` filters, backed by `idx_claims_triple`. All three
  in-Rust filters moved to SQL: a procedure precondition is now one `LIMIT 1` query
  instead of loading every claim above the confidence floor; the Gatekeeper's duplicate
  check no longer misses a duplicate past its hundredth row; and `Query` over the wire no
  longer applies `limit` *before* the triple filter, which had made `limit` mean "rows
  scanned" and dropped matches that sat past it. `Goal::expand` still evaluates edge
  preconditions in-process, on purpose: it fetches one context slice per hop, and
  per-precondition queries would trade one query for N. *shipped (#52)*
- **Promotion timing (§8 #4).** The background sweep and its configurable interval landed
  with the Janitor (#13); what was missing was §8.1's other half, the synchronous fast-track,
  so an endorsed entry waited up to `janitor.sweep_interval_minutes` (default 60) to climb.
  `Janitor::endorse_and_promote` records the endorsement and re-evaluates that one procedure
  in the same call. Sweep and fast-track share one `evaluate_procedure`, so the fast-track
  changes when an entry is judged, not how — falls still take priority, `dry_run` still
  suppresses the Janitor's own mutation, and an unauthorized endorser is refused before
  anything is written. **Nothing on the wire calls it yet**: there is no endorsement
  transport, and authoring is *deferred* below. *shipped* (#63)
- **Goal and procedure authoring over the wire.** Absent on purpose. Authoring is a §5
  gatekept, provenance-stamped *write*, not a read, and shipping it as a read-shaped
  endpoint would put the write path behind the wrong gate. Unblocked by the security
  session settling how an authoring caller is authenticated — and note that the Sybil
  defense depends on the transport setting `author` from the authenticated principal, never
  from caller input.
  *deferred*
- **Procedure and goal learning (§8 #3).** Inducing control flow and decomposition from
  experience — meaningfully harder than claim extraction, and the `Extractor` is the model.
  Waiting on a corpus of real episodes, which is exactly what the transport work (#21, #24)
  now lets accumulate. Correctly sequenced last; starting it before the corpus exists
  produces a worse inducer, not an earlier one.
  *deferred*

## Identity, trust and security

Who said it, how well we know that, and what it lets them do.

- **`IdentityProvider` port, assurance-gated tier ceilings, and the devAuth adapter.**
  *shipped* (#14)
- **Sybil-resistant corroboration via provenance diversity.** *shipped* (#18)
- **The identity port wired through the transport**, so a reporter's assurance comes from
  the provider rather than being hardcoded to `none`, plus the `X-Boswell-Auth` response
  marker. *shipped* (#27)
- **The Sybil defense measured against a running system.** `sybil_scenarios.rs` stands
  devAuth's four identities against the real store, the real corroboration path and the
  real gatekeeper. Findings are §8.3. *shipped* (#31)
- **Corroboration counts the authenticated principal, not the asserted root** (§8.3 #1).
  Nine self-rooted clones collapse onto one. *shipped* (#33)
- **devAuth's own trust gradient made reachable** (§8.3 #2 and #3). Both rules were
  correct and unit-tested, and neither had ever run end to end, because nothing could
  reach the state they governed. *shipped* (#34)
- **`min_distinct_sessions` and `min_distinct_evidence_types`, defaulting to 0** (§8.3 #4).
  Off deliberately: write and endorse stamps are minted in-process with `session_id: None`,
  so a non-zero default would make corroboration unreachable rather than stricter. Raise it
  once an authoring transport stamps sessions. *shipped* (#35)
- **The code stops claiming security it does not provide.** `enable_tls` refuses to start
  instead of printing "TLS enabled" and serving plaintext; devAuth's manifest no longer
  claims to be excluded from production builds. *shipped* (#36)
- **gRPC authentication, resolved by deletion.** The service used to check that `auth_token`
  was non-empty and nothing else — fourteen call sites, no signature verification anywhere, so
  any process that could reach the port could write to any tier by sending the string `"x"`.
  The posture question is answered ([ADR-021](../ADRs/021-gateway-is-the-security-boundary.md)):
  the gateway is the boundary, so the `auth_token` plumbing was deleted rather than verified and
  the loopback bind became a startup error rather than a recommendation. The router still mints
  its session JWT for topology discovery (ADR-019); no instance reads it. See the security
  implementation section below. *shipped* (#58)
- **devAuth becomes a test-only fixture.** Out of `boswell-server`, still driving the Sybil
  scenarios — which are the entire evidence base for §8.3 and cannot move with it. The
  consequence to settle first: with no adapter shipped, every deployment runs with no
  `IdentityProvider`, so reports stamp `Assurance::None`, ceilings sit at the floor and
  nothing ever promotes. The trust gradient is inert out of the box.
  *open*
- **Router configuration encryption.** `boswell-router/src/config.rs` reads plaintext TOML;
  [`09-router.md`](../architecture/09-router.md) files the portable encrypted config as open
  and states the argument on both sides, and
  [`16-backup-recovery.md`](../architecture/16-backup-recovery.md) notes the `age`-encrypted
  config in `10-security.md` is aspirational. *open*
- **JWT refresh.** The router issues tokens with an expiry and no refresh path; the SDK
  papers over it by reconnecting once on `Unauthenticated`. *open*

### Security design session

Held 2026-09-10. Four of the six questions are answered and recorded as
[ADR-021](../ADRs/021-gateway-is-the-security-boundary.md) and
[ADR-022](../ADRs/022-delegated-credentials.md); ADR-017 is superseded. What the session
settled:

- **The threat model.** Boswell is a memory server reached by agents acting for one
  implementer, deployed either on that implementer's machine or on a host of theirs reachable
  from the internet. Public reach is a requirement, so authentication at the boundary is a
  control rather than a preference.
- **Where the boundary is.** The HTTP gateway. The gRPC instance is inside it, binds to
  loopback by construction, and loses its `auth_token` theatre rather than gaining real
  verification. ADR-021.
- **How agents authenticate.** OIDC device-code establishes the implementer once, for months;
  the gateway trades that for an attenuable token the agent narrows locally for each subagent,
  with no issuer in the loop. ADR-022.
- **Whether Boswell ships an identity adapter.** No. Identity stays the operator's to run and
  govern; the gateway consumes OIDC rather than implementing it.

Still open, and now sharper for having a decided model to sit in:

- **Where does TLS terminate?** The standing position, recorded in
  [`README.md`](../../README.md) and in `boswell-gateway/src/config.rs`, is that TLS and
  public reach are provided by a reverse proxy or tunnel in front of Boswell — neither the
  gateway nor the instance terminates it. With the gateway now the boundary rather than one
  transport among two, this is worth revisiting deliberately rather than inheriting.
- **How are gateway API keys issued and rotated?** Today they are SHA-256 hashes in a config
  file with no issuance path. ADR-022 supplies the shape of the answer — tokens descend from a
  grant — but not the migration from the keys that exist.
- **The inert trust gradient.** With no identity adapter shipped, every deployment runs with no
  `IdentityProvider`, so reports stamp `Assurance::None`, ceilings sit at the floor and nothing
  promotes. ADR-022 answers this in principle; nothing implements it yet.
- **Should the router refuse a non-loopback bind?** ADR-021 puts it on loopback behind the
  gateway and `10-security.md`'s deployment postures assume it is there, but `RouterConfig`
  accepts `bind_address = "0.0.0.0"` and `start_server` binds it. The gRPC instance refuses
  exactly this before the socket opens. Mirroring that refusal is a decision, not a bug fix:
  an exposed router is an unauthenticated endpoint that mints session tokens and lists every
  instance endpoint. Documented in [`09-router.md`](../architecture/09-router.md) (#60, #61).

### Security implementation, following the session

- **Delete the `auth_token` plumbing and enforce the loopback bind** (ADR-021). The field is
  gone from all fourteen request messages — `reserved`, so the numbers are not reused — along
  with its fourteen `is_empty()` checks and the SDK's carrying of it. `ServerConfig` now
  resolves its bind address and refuses anything that is not loopback, the same way
  `enable_tls` refuses to serve plaintext. The router still mints its session JWT for topology
  discovery (ADR-019); it was never read by an instance and is not an authorization credential.
  *shipped* (#58)
- **OIDC verification at the gateway** (ADR-022). A `[oidc]` section names an issuer the
  operator already runs; a caller presents that provider's JWT as its bearer token and the
  gateway verifies it against JWKS it has cached, so no request costs a round trip. Keys are
  refetched when the set goes stale or a token names a key id the gateway has not seen, rate
  limited so unknown-kid tokens cannot be pointed at the provider as an amplifier. Verification
  establishes *who*: authority comes from a `[[oidc.principals]]` entry matching the token's
  `sub`, the same way an `[[api_keys]]` entry supplies it for a key, so a verified subject with
  no entry is 403 rather than 401. API keys are untouched and a JWT presented to a gateway with
  no issuer configured is just a bad key. *shipped* (#68)
- **The device-code client half.** Nothing in the repo obtains a token — an operator gets one
  from their provider by hand. The grant runs between the client and the provider with the
  gateway not in it, so this is a `boswell login` command, not gateway work. *open*
- **Attenuable tokens** (`biscuit-auth`): root token minted from a verified OIDC identity,
  attenuation for subagents, and the Datalog authorization policy. *open*
- **Revocation list.** Offline verification means a revoked grant is invisible until something
  checks; the gateway needs a revocation table keyed by token id, and every token needs a
  bounded lifetime so the list stays small. *open*
- **Corroboration resolves a token to its delegation root**, so ten subagents of one agent do
  not read as ten independent witnesses. `CONTEXT.md` already says the root is the unit of
  independence and #33 counts the authenticated principal; the two diverge the moment subagents
  hold their own tokens. *open*
- **Rewrite [`10-security.md`](../architecture/10-security.md).** It specified the mTLS model
  the session rejected. It now describes the decided design and separates three states
  explicitly — built, decided-not-built, and genuinely open — so nothing in it reads as
  deployable that is not. The four documents that described it as an unbuilt "target model"
  were corrected in the same PR: `claude-code-hooks.md` §8, the closing note in
  `http-api.md`, the backup-encryption item in `16-backup-recovery.md`, and the relationship
  line in `15-procedural-memory.md`. *shipped* (#59)
- **Correct the config table in [`09-router.md`](../architecture/09-router.md).** It was from
  the same superseded design as the old `10-security.md`: `config_path: ./router.enc`,
  `listen_address: 0.0.0.0:9000`, and a `signing_key_path` "in encrypted config". The section
  now has two tables — the keys `RouterConfig` actually parses, and what the old table
  promised with each item's real status. Whether the `age`-encrypted config survives ADR-021
  was left open, not resolved. *shipped* (#60)
- **Finish [`09-router.md`](../architecture/09-router.md).** #60 corrected the config table and
  marked six sections as unbuilt; the body still described a router that does not exist. It is
  now a description, in the shape of `10-security.md` with one category added. **Superseded**
  is the addition, and it carries mTLS session establishment, per-instance tokens validated at
  the instance, registry fingerprints and per-instance capability declaration — abandoned under
  ADR-021, not queued. Health monitoring, multiple endpoints and config encryption are
  **open**; ADR-019's client-side routing by expertise and its federated-query fallback are
  **decided, not built** — the SDK receives `expertise` and `mode` and ignores both, picking the
  first instance reported healthy. Two things got written down for the first time: `GET /health`
  is a liveness check for the router alone and always answers `healthy`, and the unmeasured
  memory and CPU figures were removed rather than replaced. *shipped* (#61)

## Transport and interfaces

Every way in: gRPC, the router, the SDK, the HTTP gateway, MCP, the CLI.

- **gRPC service — all fifteen RPCs.** *shipped*
- **Router — JWT session issuance and instance registry**, single-instance mode. *shipped*
- **Rust SDK** covering all fifteen RPCs, with connection pooling. *shipped*
- **`boswell-gateway` — the public HTTP/JSON API.** Fifteen routes, SHA-256-hashed bearer
  keys, per-key rate limiting, namespace isolation. The only component with real request
  authentication. *shipped* (#6)
- **Claude Code hooks integration** — `SessionStart` recall, `UserPromptSubmit` capture,
  backed by `POST /v1/hooks/ingest`. *shipped* (#5)
- **Procedural memory over the wire (7a)** — `QueryProcedures`, `GetProcedure`,
  `ReportOutcome`; `GET /v1/procedures`, `GET /v1/procedures/{id}`,
  `POST /v1/receipts/{id}/report`. Retrieval issues the receipt; scope is checked *before*
  it is issued. *shipped* (#21)
- **Goals and traversal over the wire (7b)** — `QueryGoals`, `GetGoal`, `Expand`;
  `GET /v1/goals[/{id}[/expand]]`. Traversal issues **no** receipt. *shipped* (#24)
- **CLI procedural-memory commands**, and the clap flag collision that was panicking
  `query`, `learn` and `forget` on every invocation. *shipped* (#26)
- **MCP tool surface for goals and procedures.** Six tools added beside the five claim
  ones: `boswell_query_goals`, `boswell_get_goal`, `boswell_expand_goal`,
  `boswell_query_procedures`, `boswell_get_procedure`, `boswell_report_outcome`. The
  JSON is the gateway's field for field, so the two surfaces render the same domain
  types the same way. `issued_to` is **not** a tool parameter — the server names the
  principal from `BOSWELL_MCP_PRINCIPAL` (default `mcp`), the way the gateway names it
  from the API key; the CLI's `--as` is the deliberate exception, since an operator is
  not the same trust as a model choosing arguments. A miss is `found: false`, not a
  JSON-RPC error. *shipped* (#67)
- **gRPC graceful shutdown.** `server.rs` now calls `serve_with_shutdown`, waiting on
  `ctrl_c` like the Janitor and Synthesizer workers. `start_server_with_shutdown` takes
  the signal as a parameter for callers with a lifecycle of their own. *shipped* (#50)
- **SDK retry with exponential backoff.** `RetryPolicy` on `BoswellClient` — three
  retries from 100ms, doubling to a 5s ceiling, with equal jitter. Applied to transient
  transport statuses (`Unavailable`, `DeadlineExceeded`, `ResourceExhausted`, `Aborted`)
  on read-only RPCs only: nothing on the wire carries an idempotency key, so a repeated
  `Assert` is a second claim and a repeated `QueryProcedures` is a second receipt.
  Re-establishing an expired session is unchanged at one attempt, under every policy.
  *shipped* (#54)
- **Un-ignore the full-stack E2E tests.** `boswell-sdk/tests/e2e_tests.rs` now stands the
  whole stack up in-process — a gRPC instance over an in-memory store and a router that
  has it registered, both on ephemeral ports — so CI exercises SDK → Router → gRPC →
  Store on every run. Nothing in the file is `#[ignore]`d. What made it possible is
  `boswell_router::start_server_with_shutdown`, mirroring the gRPC entrypoint from #50:
  before it, `start_server` never returned. *shipped* (#62)
- **REPL procedural-memory commands.** The REPL implements nine commands and omits `goal`,
  `procedure` and `validate`. Its parser is hand-rolled and positional, so this is its own
  piece of work rather than a wiring change — and it already omitted a shipped command
  (`validate`) before procedural memory existed. Not an oversight.
  *deferred*

## Background processes

The passes that run without being asked.

- **Janitor — decay, tier-TTL garbage collection, confidence-based demotion, dry-run
  mode.** *shipped*
- **Contradiction detection**, recording `Contradicts` relationships folded into confidence
  as a penalty, rate-limited. *shipped*
- **Synthesizer — clustering, prompting, derived claims via `DerivedFrom`.** Complete with
  its own test suite. The February plan called this "deferred to Phase 4+"; it has been
  done for some time. *shipped*
- **Procedure and receipt sweeps** — expiring unanswered receipts, which is the mechanism
  that actually enforces "silence is not success". *shipped* (#13, #17)

## Observability and performance

Named as a workstream because it has never had one, and that is why none of it exists.

- **Tracing in the gRPC service layer.** Every RPC opens a `debug` span named for its
  handler and closes with one event. State changes are `info`, reads and rejections
  `debug`, store failures `error` — the only place a `Status::internal`'s cause survives.
  Remembered content is never a field, and a test asserts it. *shipped* (#57)
- **Metrics export.** `GET /metrics` on the gateway, Prometheus exposition format, behind
  the `read` scope. The instance serves no HTTP (ADR-021), so the gateway scrapes the
  Janitor's counters over the new `GetMetrics` RPC on demand and renders them; nothing is
  pushed and nothing is cached. Rendered by hand — the workspace still has no Prometheus
  dependency, and at four counters a registry would be indirection over the one place the
  numbers are made. Shipped carrying no claim count, because counting then cost a full
  scan; #65 made counting cheap and added it. *shipped* (#64)
- **A claim count worth scraping.** `ClaimStore::count_claims` counts what `query_claims`
  would return without materializing it — the two share one filter builder in `SqliteStore`
  so they cannot drift, and the trait's default implementation is the old query-then-length
  so no other store had to change. `GET /v1/health` now counts instead of scanning, and
  `/metrics` carries `boswell_claims{tier}`: a gauge, not a counter, and a fact about the
  store rather than the Janitor, so it is reported whether or not a sweep loop is running.
  *shipped* (#65)
- **Benchmarks.** `crates/boswell-store/benches/claim_store.rs`, under criterion, measures
  the two paths the February plan put a number on. The number holds with room: asserting
  claims into a file-backed database runs at **~3,200/sec** median and **~1,280/sec** at
  p95 against a target of 100, and the slowest query measured — a filter matching a quarter
  of a 10,000-claim corpus — is **1.3 ms** at p95 against a target of 100 ms. A point read
  is 3.6 µs and flat from 1,000 claims to 10,000. The suite also prices #65's decision:
  `count_claims` over 10,000 claims takes 61 µs where the query-then-length it replaced
  takes 3.8 ms. CI compiles the target and runs each benchmark once, but never times it —
  a shared runner's wall-clock is noise, and a timed benchmark that gates a merge is a
  flaky test. *shipped* (#66)
- **The store never enables WAL mode**, which the benchmark above is what surfaced.
  `SqliteStore` sets no `journal_mode` or `synchronous` pragma, so every assert writes a
  rollback journal and fsyncs; [01-architecture.md](../architecture/01-architecture.md)
  names "SQLite (WAL mode)" as the storage decision and
  [16-backup-recovery.md](../architecture/16-backup-recovery.md) tells an operator to
  checkpoint the WAL as part of a snapshot. The code is the odd one out. Turning WAL on is
  a one-line pragma and a real operational change — two extra files beside the database,
  different snapshot semantics — so it is filed rather than done. *open*
- **Test wall-clock.** `test_run_cycles` slept sixty real seconds against a live
  `tokio::time::interval` — most of the workspace total. Now runs on a paused clock.
  *shipped* (#36)

## Storage portability

The "start simple, grow" path from [ADR-020](../ADRs/020-swappable-storage-backends.md).

- **Embedded SQLite adapter.** The one adapter that ships today. *shipped*
- **Harden the `ClaimStore` contract.** Async methods, and query filters pushed into
  `ClaimQuery` rather than applied after the fact. **Prerequisite for every slice below**
  — the port is not yet good enough to have a second implementation behind it, which is
  why "swappable storage" is a design intent rather than a present capability. *open*
- **PostgreSQL + pgvector adapter**, for shared, multi-agent and hosted deployments.
  *open*
- **Data-store migration tool.** Move claims, relationships, provenance and embeddings
  between adapters preserving ids, tiers, confidence and timestamps. Likely
  `boswell migrate`. *open*
- **Backup and restore tooling.** `boswell backup` / `boswell restore`, per
  [Backup & Recovery](../architecture/16-backup-recovery.md). The design exists; no code
  does. *open*

## LLM providers

The adapter layer from [ADR-015](../ADRs/015-pluggable-llm-providers.md): every subsystem
that needs a model calls one trait, and configuration decides who answers it.

- **`LlmProvider` trait, the deterministic `MockProvider`, and the local Ollama adapter.**
  *shipped*
- **Hosted adapters — OpenAI, OpenRouter, DeepSeek, Anthropic, Google Gemini.** Five
  vendors, three adapters: OpenAI, OpenRouter and DeepSeek share one wire format, so
  `OpenAiCompatProvider` serves all three and takes any other endpoint speaking it, while
  `AnthropicProvider` and `GeminiProvider` exist because their request and response shapes
  genuinely differ, not because of vendor branding. Keys arrive as constructor arguments or
  from the vendors' conventional environment variables; no provider derives `Debug`, so a
  key cannot reach a log through `{:?}`. A model that declines to answer now has its own
  error — both Anthropic and Google report a refusal as an HTTP 200, which without it reads
  as an empty answer. Request timeouts are configurable on every adapter, and one
  ignored test drives `OpenAiCompatProvider` against Ollama's own `/v1` endpoint —
  the whole request-and-parse path, no vendor account, no bill.
  *shipped* (#40, #41)
- **Per-subsystem provider configuration.** ADR-015's actual decision was that each
  subsystem — Extractor, Gatekeeper, Janitor, Synthesizer — maps to a provider
  independently. The trait supports that; nothing reads configuration to do it. The
  Extractor is handed one provider at construction and the rest have no wiring at all.
  Until this ships, "pluggable" means a Rust caller can choose, not an operator. *open*
- **Schema-constrained decoding.** `generate_structured` ignores its `schema` argument on
  every adapter, Ollama's included, and returns whatever text came back. Each vendor
  constrains decoding differently — `response_format`, `output_config.format`,
  `responseSchema`, Ollama's JSON mode — and the trait says nothing about what a `schema`
  string contains, so honoring it means first deciding that contract. The Extractor parses
  free text today and does not call it. *open*
- **Name a chat model that exists.** Every default, sample and doc pointed at
  `qwen2.5:7b`, and the shipped `config/instance.toml` had drifted from the sample the
  server itself emits — it was missing `[extraction]` entirely. *shipped* (#42)
- **Quiet hours for the background jobs.** `[janitor]`, `[synthesizer]` and
  `[contradiction]` each take an optional `run_between = "02:00-06:00"` in local time,
  so the heavy local-model work can be kept to hours nobody is using the machine. The
  window is a gate, not a schedule: the interval still says how often, and a windowed
  job polls rather than sleeping a full interval, because a twelve-hour interval landing
  at 13:00 and 01:00 would never fall inside a four-hour night window. An unparseable
  window stops the server at startup, since a window that never opens fails silently at
  2 a.m. where nobody is watching. *shipped* (#43)
- **Cost and token accounting.** Every hosted response carries usage counts and every one
  of them is discarded. Nothing in Boswell can answer what an extraction run cost. *open*
- **Streaming, tool use and multi-turn conversation.** Absent on purpose. The trait is one
  prompt in, one string out; widening it is a change to the port, which is a decision about
  what subsystems are allowed to ask for, not an adapter feature. *deferred*

---

# Appendix: the original five-phase plan (February 2026)

What follows is the roadmap as written on 2026-02-13, preserved unchanged as the record of
what the project intended at the outset. **It is historical and is not maintained.** Its
checkboxes were never ticked — thirty-six unchecked items while two phases were
essentially complete — and its five-phase structure has no place for the procedural-memory,
identity or gateway work that followed. Read it for intent, not for status.

## Plan: Boswell Cognitive Memory System - Development Roadmap

Boswell is a claim-based cognitive memory system for AI agents, built in Rust with Clean Architecture principles. This plan delivers a functional single-instance system incrementally, with 11+ components organized into 5 phases. Each phase has clear deliverables, validation criteria, and parallelizable work streams for 2-3 subagent contributors.

**Critical Decisions:**
- Single-instance MVP first; federation deferred to future phases
- Real LLM providers (Ollama) from Phase 1 for authentic testing
- Synthesizer deferred to Phase 4+ (non-critical to core operations)
- Conservative 2-3 contributor parallelization minimizes coordination overhead

---

### PHASE 1: FOUNDATION

**Goal:** Establish domain core, storage layer, and LLM integration without external API surface

**Contributors Assigned:** 2 parallel streams

#### Contributor A: Domain Core (`boswell-domain`)

1. Create crate with zero external dependencies (per ADR-004)
2. Implement core value objects:
   - `ClaimId` (ULID-based, per ADR-011)
   - `ConfidenceInterval` with `[lower, upper]` bounds (ADR-003)
   - `ProvenanceEntry` with source/timestamp/rationale
   - `Relationship` with pairwise constraint (ADR-002)
   - `Namespace` with slash-delimited validation (ADR-006)
3. Define `Claim` struct with all fields from [02-claim-model.md](../architecture/02-claim-model.md)
4. Define trait interfaces:
   - `ClaimStore` trait
   - `LlmProvider` trait with capability methods (ADR-015)
   - `Extractor`, `Synthesizer`, `Gatekeeper` traits
5. Implement deterministic confidence formula (ADR-007):
   - Recursive support network traversal
   - Source diversity weighting
   - Circular relationship protection
6. Add comprehensive property-based tests using `proptest`:
   - ULID ordering properties
   - Confidence interval invariants
   - Namespace depth validation
   - Formula convergence properties
7. XML documentation for all public types and methods

**Deliverable:** `boswell-domain` crate compiles with 100% doc coverage, passes property tests

---

#### Contributor B: Storage Layer (`boswell-store`)

1. Create SQLite schema (fill documentation gap):
   - `claims` table with ULID primary key
   - `relationships` table (pairwise only)
   - `provenance` table
   - `confidence_cache` table for fast-path values
2. Implement `ClaimStore` trait using `rusqlite`:
   - `assert_claim()` with duplicate detection
   - `query_claims()` with structured filters
   - `get_by_id()` for direct retrieval
   - Temporal queries via ULID range scans (ADR-011)
3. Integrate HNSW vector index (ADR-005):
   - Use `hnswlib-rs` or equivalent
   - Memory-mapped index for performance
   - Separate index file alongside SQLite database
4. Implement local embedding pipeline (ADR-013):
   - ONNX runtime integration with `tract` or `ort`
   - Ship with `bge-small-en-v1.5` model (384 dims)
   - Document dimension configuration
5. Implement embedding-based duplicate detection:
   - Cosine similarity threshold (document default)
   - Pre-insert check against existing claims
6. Add migration framework using `rusqlite` migrations
7. Integration tests with in-memory SQLite database

**Deliverable:** `boswell-store` crate with full CRUD operations, vector search, and passing integration tests

---

#### Phase 1 Sync Point: LLM Provider Layer (`boswell-llm`)

Both contributors collaborate once their streams complete:

1. Define `LlmProvider` trait implementation structure
2. Create `MockProvider` for deterministic testing
3. Implement `OllamaProvider` (local, no API keys needed):
   - HTTP client for Ollama API
   - Streaming response handling
   - Error handling and retries
4. Configuration structure for per-subsystem providers (ADR-015)
5. Unit tests with mock, integration tests with Ollama (conditional on availability)

**Deliverable:** `boswell-llm` crate with mock and Ollama providers

---

**Phase 1 Validation:**
- [ ] Domain core compiles with zero warnings
- [ ] All property tests pass (confidence formula, ULID ordering, namespace validation)
- [ ] Store can assert and query claims with confidence computation
- [ ] Embedding pipeline produces consistent vectors
- [ ] Vector search returns semantically similar claims
- [ ] Ollama provider successfully calls local LLM
- [ ] Full test suite runs in <10 seconds

---

### PHASE 2: CORE OPERATIONS

**Goal:** Expose gRPC API for basic operations, enable client interactions

**Contributors Assigned:** 3 parallel streams

#### Contributor A: gRPC Service Layer (`boswell-grpc`)

1. Define `.proto` files for API surface (fill documentation gap):
   - `AssertRequest/Response` with optional tier targeting
   - `QueryRequest/Response` with fast/deliberate mode flag
   - `LearnRequest/Response` for bulk loading (ADR-012)
   - `ForgetRequest/Response` for eviction marking
   - `HealthCheckRequest/Response`
2. Generate Rust code using `tonic-build`
3. Implement service handlers:
   - Route to `ClaimStore` for storage operations
   - Handle provenance recording
   - Apply tier-based validation
4. Add authentication stubs (token validation placeholder)
5. Error mapping from domain errors to gRPC status codes
6. Server initialization with TLS configuration (ADR-017)

**Deliverable:** `boswell-grpc` crate with functional gRPC server

---

#### Contributor B: Router (`boswell-router`)

1. Implement session management (ADR-019):
   - Session token generation (signed JWT or similar)
   - Stateless session validation
   - Token expiry handling
2. Create instance registry:
   - Single-instance mode (self-registration)
   - Health status tracking (stub for now)
   - Encrypted configuration storage
3. Implement session establishment endpoint:
   - Accept domain hints from clients
   - Return instance endpoints and tokens
4. Add health check aggregation
5. Configuration file parsing (TOML) - fill documentation gap
6. CLI bootstrapping command (initial instance setup)

**Deliverable:** `boswell-router` crate with single-instance session management

---

#### Contributor C: Client SDK (`boswell-sdk`)

1. Create Rust SDK wrapping gRPC calls:
   - Session establishment via Router
   - Direct instance communication after session
   - Automatic token inclusion in requests
2. Implement API methods:
   - `assert()`, `query()`, `learn()`, `forget()`
   - Synchronous interface (async can come later)
3. Error handling with typed errors
4. Connection pooling for gRPC channels
5. Example code for common operations
6. Integration tests against running server

**Deliverable:** `boswell-sdk` crate with full API coverage

---

**Phase 2 Validation:**
- [ ] gRPC server starts and responds to health checks
- [ ] Router issues valid session tokens
- [ ] SDK successfully establishes session and routes to instance
- [ ] End-to-end test: SDK → Router → gRPC → Store → Response
- [ ] Claims can be asserted and queried via SDK
- [ ] Bulk `learn()` operation works with batches of pre-formatted claims
- [ ] TLS connections succeed with test certificates

---

### PHASE 3: INTELLIGENT OPERATIONS

**Goal:** Add LLM-backed operations (Extractor, Gatekeeper)

**Contributors Assigned:** 2 parallel streams

#### Contributor A: Extractor (`boswell-extractor`)

1. Design LLM prompts for text → claims conversion (fill documentation gap):
   - System prompt defining claim structure
   - Examples of good claim extraction
   - Instructions for splitting compound statements
2. Implement `Extractor` trait:
   - Accept unstructured text blocks
   - Call LLM provider with prompt
   - Parse LLM response into `Claim` structs
   - Handle malformed responses gracefully
3. Integrate with `ClaimStore` duplicate detection:
   - Check embeddings before insertion
   - Decide merge vs. new claim policy
4. Record provenance linking to source text
5. Add Extract operation to gRPC API:
   - `ExtractRequest` with text and optional namespace/tier
   - Synchronous blocking operation
6. Unit tests with mock LLM, integration tests with Ollama
7. Measure performance: claims extracted per second

**Deliverable:** `boswell-extractor` crate with gRPC integration

---

#### Contributor B: Gatekeeper (`boswell-gatekeeper`)

1. Define tier promotion evaluation prompts (fill documentation gap):
   - Per-tier criteria (ephemeral→task, task→project, project→permanent)
   - Advocacy tuple structure
   - Expected response format (accept/downgrade/reject + rationale)
2. Implement `Gatekeeper` trait:
   - Accept promotion requests with advocacy tuples
   - Call LLM with claim content + relationships + tier target
   - Parse decision and reasoning
3. Record gatekeeper reasoning as provenance (ADR-008)
4. Implement promotion request queue (deferred evaluation):
   - Background thread polling queue
   - Rate limiting to prevent LLM overload
5. Add Promote operation to gRPC API:
   - `PromoteRequest` with claim ID and target tier
   - Async response model (request accepted, decision later)
6. Create tier-specific configuration:
   - Different LLM models per tier boundary
   - Configurable evaluation criteria
7. Integration tests covering accept/downgrade/reject paths

**Deliverable:** `boswell-gatekeeper` crate with gRPC integration

---

**Phase 3 Validation:**
- [ ] Extractor converts sample text into structured claims
- [ ] Extracted claims have proper provenance linking to source
- [ ] Duplicate detection prevents redundant storage
- [ ] Gatekeeper evaluates promotion requests within 5 seconds (with Ollama)
- [ ] Gatekeeper reasoning is stored and retrievable
- [ ] Promotion rejections preserve claims in original tier
- [ ] End-to-end test: Extract text → Assert to ephemeral → Promote to task → Gatekeeper accepts

---

### PHASE 4: BACKGROUND PROCESSES

**Goal:** Add automated maintenance (Janitors) and optional synthesis

**Contributors Assigned:** 2-3 parallel streams

#### Contributor A: Core Janitors (`boswell-janitor`)

1. Create janitor framework:
   - Background thread with configurable interval
   - Graceful shutdown handling
   - Per-janitor enable/disable configuration
2. Implement **Staleness Janitor**:
   - Apply confidence decay based on half-life model (ADR-009)
   - Update `confidence_cache` table
   - No LLM required (deterministic)
3. Implement **GC Janitor**:
   - Query forgotten claims past retention period
   - Hard delete from SQLite and HNSW index
   - Batch deletions for efficiency
4. Implement **Confidence Recomputation Janitor**:
   - Identify claims with stale cached confidence
   - Recompute via support network traversal
   - Update cache
5. Integration with gRPC server: janitors start with server
6. Observability: log janitor runs and metrics (claims processed, deleted)

**Deliverable:** `boswell-janitor` crate with 3 core janitors operational

---

#### Contributor B (Optional): Advanced Janitors

1. Implement **Contradiction Janitor** (high complexity):
   - Query pairs of claims with overlapping namespaces/subjects
   - Call LLM for semantic contradiction detection
   - Record Challenge relationships
   - Rate-limited to prevent LLM overload
2. Implement **Tier Migration Janitor**:
   - Identify demotion candidates (low confidence, no recent access)
   - Downgrade tiers automatically or queue for review
3. Configuration for janitor aggressiveness
4. Integration tests with mock LLM for contradiction detection

**Deliverable:** `boswell-janitor` crate with 5 janitor types

---

#### Contributor C (Phase 4b): Synthesizer (`boswell-synthesizer`)

*Deferred to Phase 4b - start only after Phase 4a validation*

1. Design synthesis prompts (fill documentation gap):
   - Instruct LLM to identify patterns across claim clusters
   - Generate higher-order derived claims
   - Specify `derived_from` relationship format
2. Implement `Synthesizer` trait:
   - Cluster claims by namespace/semantic similarity
   - Sample clusters for LLM analysis
   - Parse emergent insights into new claims
3. Create derived claims with proper provenance:
   - `derived_from` relationships pointing to source claims
   - Confidence inherited/computed from sources
4. Background scheduling (configurable, not triggered by API)
5. Prevent synthesis loops:
   - Max derivation depth
   - Skip already-synthesized clusters
6. Integration tests with small claim sets

**Deliverable:** `boswell-synthesizer` crate with scheduled synthesis

---

**Phase 4 Validation:**
- [ ] Staleness janitor reduces confidence over time according to half-life
- [ ] GC janitor deletes forgotten claims after retention period
- [ ] Confidence recomputation janitor keeps cache accurate
- [ ] Contradiction janitor identifies known contradictory claim pairs
- [ ] Tier migration janitor demotes stale low-confidence claims
- [ ] Synthesizer produces at least one derived claim from test data
- [ ] Janitors run on schedule without blocking gRPC operations
- [ ] System remains responsive under janitor load

---

### PHASE 5: CLIENT INTEGRATION & POLISH

**Goal:** Expose via MCP, add CLI tooling, optimize performance

**Contributors Assigned:** 2 parallel streams

#### Contributor A: MCP Server (`boswell-mcp`)

1. Implement MCP protocol server:
   - Expose all operations as MCP tools
   - Tool schemas for each operation (Assert, Query, Extract, Learn, Promote, Forget, Reflect)
   - Session management wrapper using SDK
2. Create tool definitions:
   - Clear descriptions for LLM consumption
   - Parameter schemas with validation
   - Example invocations
3. Add Reflect operation:
   - LLM-backed narrative synthesis from query results
   - Configurable depth/breadth tradeoffs
4. Error handling and user-friendly messages
5. Integration with Claude Desktop, Cursor, etc.
6. Documentation for MCP setup

**Deliverable:** `boswell-mcp` crate with full MCP server implementation

---

#### Contributor B: CLI & Operations Tooling (`boswell-cli`)

1. Create admin commands:
   - `boswell init` - bootstrap new instance
   - `boswell serve` - start server with config
   - `boswell reindex` - rebuild HNSW index (ADR-014)
   - `boswell health` - instance health check
   - `boswell backup` / `boswell restore` - SQLite operations
2. Interactive claim exploration:
   - `boswell query` - query with filters
   - `boswell inspect <claim-id>` - view claim details + relationships
   - `boswell graph <claim-id>` - visualize support network
3. Bulk operations:
   - `boswell import` - load from JSON/CSV
   - `boswell export` - dump claims
4. Configuration validation:
   - `boswell config check` - validate TOML
5. CLI help text and man pages
6. Shell completion scripts (bash/zsh)

**Deliverable:** `boswell-cli` crate with comprehensive admin tooling

---

#### Contributor C (Optimization - ongoing):

1. Performance benchmarking:
   - Assert throughput (claims/sec)
   - Query latency (p50/p95/p99)
   - Vector search recall@k
   - Confidence computation time
2. Identify and optimize hot paths:
   - Profile with `cargo flamegraph`
   - Optimize SQL queries (EXPLAIN ANALYZE)
   - Tune HNSW parameters (M, efConstruction)
   - Cache frequently-accessed claims
3. Add observability:
   - Structured logging with `tracing`
   - Prometheus metrics export
   - Trace sampling for distributed tracing
4. Memory profiling and leak detection
5. Document performance characteristics and tuning guide

**Deliverable:** Performance report and optimization recommendations

---

**Phase 5 Validation:**
- [ ] MCP server connects to Claude Desktop and exposes all tools
- [ ] CLI can bootstrap instance, assert claims, query, and inspect
- [ ] Backup/restore preserves all data including vector index
- [ ] Reindex operation successfully rebuilds HNSW index
- [x] Performance benchmarks meet targets from [01-architecture.md](../architecture/01-architecture.md) (#66)
- [x] System handles 1000+ claims without degradation (#66 — measured to 10,000; point
      reads are flat across that range and filtered queries scale with the size of the
      result, not of the corpus)
- [x] Metrics exported and viewable in Prometheus (#64)

---

### CROSS-PHASE REQUIREMENTS

All contributors must adhere to:

#### 1. Testing Standards

- Unit tests for all business logic
- Integration tests for component boundaries
- Property-based tests for invariants
- BDD/Gherkin tests for user-facing operations (using `cucumber-rust`)
- Minimum 80% code coverage

#### 2. Documentation Standards

- Rustdoc comments for all public items
- Module-level documentation explaining purpose
- Examples in docs for common operations
- Architecture decision updates when deviating from ADRs

#### 3. Code Quality

- No file exceeds 300 lines (refactor if needed)
- All `clippy` lints pass at `warn` level
- Run `cargo fmt` before commits
- No `unwrap()` or `panic!()` in production code paths

#### 4. Security

- Never commit secrets or test certificates to git
- Use environment variables for sensitive config
- Validate all inputs at API boundaries
- Follow Rust memory safety guidelines

#### 5. Coordination

- Daily sync on completed work and blockers
- Update shared task board with progress
- Create GitHub issues for discovered gaps/ambiguities
- Document breaking changes in CHANGELOG.md

---

### RISK MITIGATION

#### 1. LLM Quality Risk

- Maintain prompt versioning
- A/B test prompt variations
- Collect failure cases for refinement
- Support multiple LLM providers for fallback

#### 2. Confidence Formula Risk

- Start with conservative parameters
- Instrument heavily for debugging
- Create visualization tools for support networks
- Plan for formula versioning and migration

#### 3. Performance Risk

- Benchmark early and often
- Profile before optimizing
- Document hardware requirements
- Plan for horizontal scaling (Phase 6+)

#### 4. Coordination Risk

- Clear phase gates prevent premature dependencies
- Each contributor owns complete vertical slices
- Sync points minimize integration conflicts

---

### SUCCESS CRITERIA

**Phase 1:** Foundation components compile, tests pass  
**Phase 2:** End-to-end claim lifecycle (assert → query → retrieve)  
**Phase 3:** Intelligent operations (extract text, evaluate promotions)  
**Phase 4:** Automated maintenance (decay, GC, synthesis)  
**Phase 5:** Production-ready deployment (MCP, CLI, monitoring)

**Final Deliverable:** A single-instance Boswell deployment that:
- Accepts unstructured text and extracts claims
- Stores claims with embeddings and confidence intervals
- Queries semantically with vector search
- Applies tier-based lifecycle management
- Evaluates promotion requests via gatekeeper
- Synthesizes emergent insights in background
- Exposes all functionality via MCP and CLI
- Processes 100+ assertions/sec, queries <100ms p95 *(measured in #66; met with roughly
  30x and 80x headroom respectively)*

### Backlog / Future Work

Identified but not yet scheduled into a phase.

- **Data-store migration tool.** Move all memories — claims, relationships, provenance, and
  embeddings — between storage adapters (e.g. SQLite → Postgres + pgvector), preserving ids,
  tiers, confidence, and timestamps. Supports the "start simple, grow" path
  ([ADR-020](../ADRs/020-swappable-storage-backends.md); README *Choosing Your Data Store*).
  Likely a `boswell migrate` CLI subcommand.
- **PostgreSQL + pgvector storage adapter.** An optional `ClaimStore` implementation for
  shared / multi-agent / hosted deployments ([ADR-020](../ADRs/020-swappable-storage-backends.md)).
  Prerequisite: harden the `ClaimStore` contract (async methods; push query filters into
  `ClaimQuery`).
- **Backup / restore tooling.** `boswell backup` / `boswell restore` per
  [Backup & Recovery](../architecture/16-backup-recovery.md).
