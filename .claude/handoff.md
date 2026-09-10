---
status: continue
item: gRPC graceful shutdown — shipped in #50
updated: 2026-09-10T21:05:00Z
---

# Handoff

The session that wrote this is gone. You are reading the only thing that survived it.
Git history already records *what was done* — this file exists for what git cannot show:
intent, dead ends, and the next move.

## Current item

None in flight. The last session took *gRPC graceful shutdown* from Transport and shipped
it as PR #50, branch `feat/grpc-graceful-shutdown`, commit 134bda6. The roadmap slice is
marked *shipped (#50)*.

**Check #50 before anything else.** It was opened with CI running and this file was
written before the result was known. If it merged, the branch is spent and you start
fresh. If CI is red, fixing it is your item — do not start a new slice on top of it.

## State

Branch `feat/grpc-graceful-shutdown` at 134bda6, one commit ahead of `main` (ed44f99).
Tree clean. All three CI commands passed locally before the push.

## What #50 actually did

- `server.rs` calls `serve_with_shutdown`, not `.serve(addr)`. The old call never
  returned, so an instance could only be stopped by killing the process.
- `start_server_with_shutdown` is the new full-parameter entrypoint and takes the signal
  as an argument; the three existing entrypoints delegate to it with `ctrl_c`. **No
  existing signature changed**, so no caller moved — `boswell-server/src/lib.rs:237` still
  calls `start_server_with_identity` and now gets `ctrl_c` for free.
- A failure to install the signal handler resolves the future (server stops) and reports
  on stderr. Propagating would refuse to start; pending forever would ignore the one
  signal an operator reaches for.
- `tokio`'s `signal` feature is declared in `boswell-grpc`'s manifest, and `net`/`time` in
  its dev-dependencies.

## Tried and rejected

- **Spawning the server on a `tokio::task` in the tests.** `start_server_*` returns
  `Result<(), Box<dyn std::error::Error>>`, and `Box<dyn Error>` is not `Send`, so the
  `JoinHandle` will not compile. Wrapping the call in an async block that maps the error
  to `String` does fix it, but the shape that survived is simpler: await the *server* on
  the test task and spawn the *trigger*.
- **Binding the test server to port 0 and asking it what it got.**
  `serve_with_shutdown` takes a `SocketAddr` and returns nothing until it stops, so there
  is no way to read back the ephemeral port. Hence the `free_port()` helper, which binds
  `127.0.0.1:0`, reads `local_addr().port()`, and drops the listener. That races with
  another process taking the port in the gap; a hard-coded port races every other run of
  the suite, which is worse.
- **Firing the shutdown signal after a fixed `sleep`.** A pass would not prove the server
  ever bound. The test dials the port in a retry loop and signals on the first successful
  connection instead, so a pass means bound → served → stopped.

## Next step

Pick a fresh slice from `docs/development/roadmap.md`. Assessed but not taken, in rough
order of ratio:

- **Push the triple match into SQL** (Procedural memory), marked in place at
  `crates/boswell-store/src/procedure_store.rs:351`. Self-contained; the smallest item
  now that shutdown is closed.
- **SDK retry with exponential backoff** (Transport). Today it reconnects once on
  `Unauthenticated` and gives up. Not researched.
- **Delete the `auth_token` plumbing and enforce the loopback bind** (ADR-021). Decided,
  not researched — fourteen `is_empty()` call sites in `boswell-grpc`, plus the router's
  unread JWT and the SDK carrying it. Bigger than one session may hold; it spans grpc,
  router, sdk and server config. The `auth_token.is_empty()` checks were deliberately left
  untouched by both #48 and #50; that deletion is its own slice.
- **Un-ignore the full-stack E2E tests** (Transport). `boswell-sdk/tests/e2e_tests.rs`
  needs manually started router and gRPC servers. #50 makes this *more* tractable than it
  was — `start_server_with_shutdown` is exactly the handle a test harness needs to stand a
  server up and tear it down — but the router still has no equivalent.
- **MCP tool surface for goals and procedures** (Transport). The largest feature lag: five
  claim-only tools against fifteen gateway routes. Not sized.

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
- Full workspace `cargo test` runs in well under a minute now and `clippy` in ~16s warm.
  Neither is a reason to skip the verify step.
- `cargo fmt` will rewrap a long `#[error("…")]` attribute onto its own lines. Run it
  before committing rather than hand-wrapping.
- PR numbers are shared with issues; `gh pr list --state all --limit 1` gives the last one
  used, and the next PR gets the next number. That is how #48 was cited in the roadmap
  *before* the PR existed.
- `main` is protected. Everything lands by PR, including a handoff-only change, so a
  session that means to record something for the next one must budget context for the
  branch, the PR, and the CI wait. Do not leave that to the last thousand tokens.
- The measured context number runs a few thousand above what `/context` reports, because
  it counts the raw cached input the API bills for. Treat it as the conservative figure;
  it is the one to compare against the budget.
- Tests that stand a server up belong in `server.rs`'s `mod tests`, not in a `tests/`
  directory — `boswell-grpc` has no integration-test directory and did not grow one for
  #50. `free_port()` and `in_memory_store()` live there and are worth reusing.
- The workspace pins `tokio = { features = ["full"] }`, so a per-crate `features = [...]`
  list adds nothing that is not already resolved. Declare features anyway; the list is
  documentation of what the crate actually uses, not a build constraint.

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
