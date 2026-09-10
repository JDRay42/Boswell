---
status: continue
item: Push the triple match into SQL — shipped in #52
updated: 2026-09-10T23:05:00Z
---

# Handoff

The session that wrote this is gone. You are reading the only thing that survived it.
Git history already records *what was done* — this file exists for what git cannot show:
intent, dead ends, and the next move.

## Current item

None in flight. The last session took *Push the triple match into SQL* from Procedural
memory and shipped it as PR #52, branch `feat/claim-query-triple-filter`, commit 6808f16.
CI was green; it merged as 54a76e6 and the branch is deleted. Nothing is waiting on you.

## State

`main` at 54a76e6, clean, nothing in flight.

## What #52 actually did

- `ClaimQuery` grew `subject`, `predicate`, `object: Option<String>`, wired into
  `SqliteStore::query_claims` as exact `=` filters and backed by a new composite index
  `idx_claims_triple ON claims(subject, predicate, object)`.
- The item as written named one call site. There were **three**, and two of them were
  carrying bugs, not just doing extra work:
  - `procedure_store::precondition_holds` — the named one. Now one `LIMIT 1` query.
  - `gatekeeper/validator.rs` duplicate check — asked for `limit: Some(100)` rows
    matching only namespace and tier, then compared triples in Rust. A duplicate past
    the hundredth row was never seen and the write went through.
  - `grpc/service.rs::query` — passed `limit` to the store and filtered the triple out of
    the *results*, so `limit` meant "rows scanned". A caller filtering by subject could
    get an empty response while matches existed. This was the user-visible one.
- `schema.sql` is `execute_batch`ed in full on every open, all statements
  `IF NOT EXISTS`, so the new index reaches existing databases with no migration step.
  `run_migrations()` was not touched.
- `synthesizer.rs` and `validator.rs` had exhaustive `ClaimQuery` literals that broke the
  build; both moved to `..ClaimQuery::default()` so the next field addition does not.

## Tried and rejected

- **Pushing the triple into `Goal::expand` as well** (`goal_store.rs:230`). Rejected on
  the merits, not for scope. `expand` deliberately fetches *one* context slice per hop at
  the lowest precondition floor and evaluates every edge check against it in-process.
  Per-precondition SQL would turn one query into N. The in-Rust filter there is the right
  shape; do not "finish the job" by changing it.
- **Scoping preconditions to a namespace while the query was being rewritten.** The old
  in-Rust match compared only the triple, across every namespace. Adding a namespace
  filter would have been a silent behavior change riding along on a performance fix.
  Left as-is; if namespace scoping is wanted it is its own decision.
- **Normalizing the gRPC `QueryFilter`'s triple fields with `.filter(|s| !s.trim().is_empty())`,
  the way `source_type` is normalized.** The Rust filter being replaced treated an empty
  string as a literal that matches nothing. Passing it through verbatim preserves that.
  Changing it is a wire-behavior change and does not belong in this commit.

## Next step

Pick a fresh slice from `docs/development/roadmap.md`. Assessed but not taken, in rough
order of ratio:

- **SDK retry with exponential backoff** (Transport). Today it reconnects once on
  `Unauthenticated` and gives up. Not researched. Probably the smallest item now.
- **Delete the `auth_token` plumbing and enforce the loopback bind** (ADR-021). Decided,
  not researched — fourteen `is_empty()` call sites in `boswell-grpc`, plus the router's
  unread JWT and the SDK carrying it. Bigger than one session may hold; it spans grpc,
  router, sdk and server config. #48, #50 and #52 all deliberately left the
  `auth_token.is_empty()` checks untouched; that deletion is its own slice.
- **Un-ignore the full-stack E2E tests** (Transport). `boswell-sdk/tests/e2e_tests.rs`
  needs manually started router and gRPC servers. #50's `start_server_with_shutdown` is
  exactly the handle a harness needs, but the router still has no equivalent.
- **Promotion timing (§8 #4).** Promotion into a Janitor-style background sweep with a
  synchronous fast-track for authority endorsements. §8.1 already picked the shape;
  engineering, not research.
- **MCP tool surface for goals and procedures** (Transport). The largest feature lag:
  five claim-only tools against fifteen gateway routes. Not sized.
- **Semantic intent match for procedure and goal retrieval.** Still open, still the
  sibling of the item #52 closed. Marked at `procedure_store.rs:295`, `goal_store.rs:114`,
  `schema.sql:185`. Needs the embedding path (ADR-005), so it is research, not plumbing.

## Open questions

None. Nothing here needs a decision before work can start.

## Environment notes

- Verify with all three, matching CI exactly: `cargo fmt --all -- --check`,
  `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.
  CI builds default features only, so feature-gated code is neither built nor tested.
- Tests needing a live Ollama server are `#[ignore]`d. Run them with
  `cargo test -- --ignored` when the work touches embeddings or LLM calls.
- Building `boswell-grpc` needs `protoc` on PATH.
- `git fetch` before branching. Branching off a stale local `main` has bitten this repo
  before — a just-merged PR was missing and it surfaced as an unrelated missing field.
- Full workspace `cargo test` runs in well under a minute and `clippy` in ~16s warm.
  Neither is a reason to skip the verify step. CI itself takes ~2m30s.
- `cargo fmt` will rewrap a long `#[error("…")]` attribute onto its own lines. Run it
  before committing rather than hand-wrapping.
- PR numbers are shared with issues; `gh pr list --state all --limit 1` gives the last one
  used, and the next PR gets the next number. That is how #48 and #52 were cited in the
  roadmap *before* the PR existed.
- `main` is protected. Everything lands by PR, including a handoff-only change, so a
  session that means to record something for the next one must budget context for the
  branch, the PR, and the CI wait. Do not leave that to the last thousand tokens.
- The measured context number runs a few thousand above what `/context` reports, because
  it counts the raw cached input the API bills for. Treat it as the conservative figure;
  it is the one to compare against the budget.
- Tests that stand a server up belong in `server.rs`'s `mod tests`, not in a `tests/`
  directory — `boswell-grpc` has no integration-test directory. `free_port()` and
  `in_memory_store()` live there and are worth reusing.
- The workspace pins `tokio = { features = ["full"] }`, so a per-crate `features = [...]`
  list adds nothing that is not already resolved. Declare features anyway; the list is
  documentation of what the crate actually uses, not a build constraint.
- Adding a field to a widely-constructed domain struct like `ClaimQuery` is a
  five-crate compile break. `cargo clippy --workspace --all-targets` finds every site in
  one pass; work through them before touching tests. Several such literals are now
  `..Default::default()` precisely so the next addition is cheaper.
- gRPC service tests against a real store use the `sqlite_service()` / `assert_one()`
  helpers around line 1260 of `service.rs`. `test_query_by_source_type` is the closest
  template for anything exercising `query`.
- To prove a new regression test actually bites, `cp` the file to `/tmp`, revert the one
  line that fixes the bug, run the single test, and restore. It costs one incremental
  build and it is the difference between a test and decoration.

## Do not re-investigate

- The roadmap is the backlog. Design lives in `docs/architecture/`, decisions in
  `docs/ADRs/`; neither carries status. `CONTRIBUTING.md` requires the roadmap be updated
  in the same PR as the work it describes.
- Where docs and code disagree about behavior, the code is wrong by default. Fix the
  code, not the doc.
- Security posture was settled on 2026-09-10 in ADR-021 and ADR-022. Read them; do not
  re-derive the decisions.
- Nothing under `.claude/` is tracked by git except this file and
  `backlog-loop.conf`. `settings.local.json` is ignored globally, and `.claude/logs/`
  self-ignores.
- The loop driver and the `/pickup` skill live in `~/.claude/scripts/backlog-loop.sh` and
  `~/.claude/skills/pickup/`, deliberately outside this repo because they are not
  Boswell-specific. The consequence is that they are unversioned and invisible from here:
  if the loop starts behaving differently, that is where to look, and nothing in this
  repo's history will explain the change.
- The skill's rule against stacking a branch on an unmerged one, and its rule against
  promising post-exit work, both came from watching the first real iteration do exactly
  those two things. They are not speculative; do not relax them.
- Assurance-based *tier ceilings* (`climb_ceiling`, `EvidenceType::tier_ceiling`) are the
  promotion path for procedures and provenance, not claim assert. They are a different
  mechanism from the tier/confidence floor #48 wired in; do not conflate them.
- `confidence_from_proto` in `crates/boswell-grpc/src/conversions.rs` already rejects
  out-of-range and inverted confidence bounds, so the Gatekeeper's
  `validate_confidence_bounds` would be redundant on the wire path.
- `serve_with_shutdown` is wired and tested. Shutdown is graceful in tonic's sense: the
  listener closes, in-flight requests finish, then the future returns. There is no
  deadline on a hung handler — that is documented, deliberate, and a caller's job via
  `tokio::time::timeout`.
- `boswell-server` has no signal handling of its own to conflict with the server's
  `ctrl_c`; `main.rs` just awaits `run(config)`.
- Roadmap line ~626, "Graceful shutdown handling", is the *Janitor's* framework item under
  the phase plan. It is not the transport slice #50 closed; do not re-open it on that
  basis.
- The `claims` table's `subject`, `predicate` and `object` columns carry no `COLLATE`, so
  SQLite `=` on them is a byte comparison — identical to Rust `==`. #52 depends on this;
  it was checked against `schema.sql`, not assumed.
- Every in-Rust triple filter over claims is gone as of #52. If you find yourself writing
  `.filter(|c| c.subject == ...)`, use the `ClaimQuery` fields instead.
