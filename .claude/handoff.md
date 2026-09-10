---
status: continue
item: SDK retry with exponential backoff — shipped in #54
updated: 2026-09-10T23:55:00Z
---

# Handoff

The session that wrote this is gone. You are reading the only thing that survived it.
Git history already records *what was done* — this file exists for what git cannot show:
intent, dead ends, and the next move.

## Current item

None in flight. The last session took *SDK retry with exponential backoff* from Transport
and shipped it as PR #54, branch `feat/sdk-retry-backoff`, commit 3a27865. CI was green;
it merged as 0398fb3 and the branch is deleted. Nothing is waiting on you.

## State

`main` at 0398fb3, clean, nothing in flight.

## What #54 actually did

- New `crates/boswell-sdk/src/retry.rs`: `RetryPolicy` (public), `Idempotency` (public),
  `RetryState` / `RetryAction` (crate-private). Default policy is three retries from
  100ms, doubling to a 5s ceiling, equal jitter. `BoswellClient::with_retry_policy` and
  `retry_policy()` are the knobs; `RetryPolicy::none()` turns the backoff path off.
- The fourteen hand-rolled `let mut retried = false;` loops in `client.rs` now each end
  in one `Err(e) => self.handle_retry(e, &mut retry, Idempotency::…).await?` arm.
  `handle_retry` is the single place that reconnects, sleeps, or gives up.
- **Backoff applies only to `Idempotency::Safe` calls.** Nothing on the wire carries an
  idempotency key, so a repeated `Assert` is a second claim. Marked `Unsafe`: `assert`,
  `learn`, `forget`, `extract`, `query_procedures`, `get_procedure`, `report_outcome`.
  `get_procedure` and `query_procedures` are `Unsafe` because retrieval issues the
  receipt — checked in `service.rs`, not assumed. `expand` is `Safe`; traversal issues no
  receipt.
- Retryable statuses are `Unavailable`, `DeadlineExceeded`, `ResourceExhausted`,
  `Aborted`. `Internal` and `Unknown` are excluded on purpose.
- Session reconnect is unchanged: exactly one attempt, for every RPC, under every policy
  including `none()`. It does not spend the backoff budget — there is a test for that.
- SDK test count went 1 → 14. `client.rs` gained a `mod tests` (it had none) that points
  a client at a closed port via `connect_lazy` and asserts on elapsed time.

## Tried and rejected

- **Adding a `rand` dependency for jitter.** Nothing in the workspace pulls `rand` in, and
  backoff jitter does not need an RNG — it needs clients that failed together to stop
  waking together. `jitter_fraction()` uses `SystemTime::now().subsec_nanos()`. If you
  find yourself reaching for `rand` here, the reason has to be something other than this.
- **Full jitter (`rand(0, base)`), the AWS-blog default.** It permits a near-zero first
  retry, which defeats backing off at all on the attempt that matters most. Equal jitter
  (`base/2 + rand(0, base/2)`) is what shipped.
- **Retrying mutating RPCs on `Unavailable` anyway,** on the theory that tonic's
  `Unavailable` almost always means the request never reached a handler. It does not
  always mean that: a server that processed the write and then died mid-response produces
  the same status. Without an idempotency key on the wire the client cannot tell the two
  apart, so it does not try.
- **Making `handle_retry` take a closure over the RPC call** to remove the `loop` from all
  fourteen sites. The closure needs `&mut self.grpc_client` while `reconnect()` needs
  `&mut self`; getting that past the borrow checker means either an `Arc<Mutex<_>>` around
  the channel or a macro. Both cost more than the fourteen four-line loops they remove.
- **Hand-wrapping the `Idempotency::Unsafe` call-site arms.** `Unsafe` is two chars longer
  than `Safe` and pushes past 100 columns, so `cargo fmt` expands those seven arms to a
  block. That is fmt's output, not a style choice; leave it.

## Next step

Pick a fresh slice from `docs/development/roadmap.md`. Assessed but not taken, in rough
order of ratio:

- **Delete the `auth_token` plumbing and enforce the loopback bind** (ADR-021). Decided,
  not researched — fourteen `is_empty()` call sites in `boswell-grpc`, plus the router's
  unread JWT and the SDK carrying it. Bigger than one session may hold; it spans grpc,
  router, sdk and server config. #48, #50, #52 and #54 all deliberately left the
  `auth_token.is_empty()` checks untouched; that deletion is its own slice.
- **Tracing in the gRPC service layer** (Observability). `service.rs` contains zero
  `tracing::` calls while `boswell-server` initialises `tracing-subscriber`. Mechanical,
  well-bounded, and probably the smallest item on the board now.
- **Un-ignore the full-stack E2E tests** (Transport). `boswell-sdk/tests/e2e_tests.rs`
  needs manually started router and gRPC servers. #50's `start_server_with_shutdown` is
  exactly the handle a harness needs, but the router still has no equivalent.
- **Promotion timing (§8 #4).** Promotion into a Janitor-style background sweep with a
  synchronous fast-track for authority endorsements. §8.1 already picked the shape;
  engineering, not research.
- **MCP tool surface for goals and procedures** (Transport). The largest feature lag:
  five claim-only tools against fifteen gateway routes. Not sized.
- **Semantic intent match for procedure and goal retrieval.** Still open. Marked at
  `procedure_store.rs:295`, `goal_store.rs:114`, `schema.sql:185`. Needs the embedding
  path (ADR-005), so it is research, not plumbing.

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
- `cargo fmt` will rewrap a long `#[error("…")]` attribute onto its own lines, and will
  expand a match arm that crosses 100 columns into a block. Run it before committing
  rather than hand-wrapping; then re-run `--check` to confirm.
- PR numbers are shared with issues; `gh pr list --state all --limit 1` gives the last one
  used, and the next PR gets the next number. That is how #48, #52 and #54 were cited in
  the roadmap *before* the PR existed. It has been right every time so far.
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
  build and it is the difference between a test and decoration. #54 did this for its
  timing test by stubbing `RetryAction::Backoff` to `Fail`.
- A repetitive mechanical edit across many call sites (#54 rewrote fourteen) is better
  done with a throwaway `python3` heredoc that regex-substitutes and *prints the count
  and the list of sites it touched*. The printed list is the check that no site was
  silently missed; `cargo build` alone will not tell you.
- SDK client tests do not need a server. `BoswellClient`'s fields are private but
  `mod tests` is inside `client.rs`, so a test can set `session_token` and `grpc_client`
  directly and point a `connect_lazy` channel at `http://127.0.0.1:1`. The first RPC then
  fails with `Unavailable` — the exact status the retry path cares about.

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
- `Goal::expand` (`goal_store.rs:230`) filters preconditions in Rust on purpose: it
  fetches one context slice per hop and evaluates every edge check against it. Pushing
  that into SQL would turn one query into N. #52 rejected this deliberately; it is the
  right shape.
- The SDK's per-RPC `Idempotency` markings in `client.rs` were derived from what each
  handler actually does, including checking `service.rs` for receipt issuance. Do not
  re-derive them. If you make an RPC idempotent on the wire, change its marking then.
