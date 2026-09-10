---
status: continue
item: Tier validation on the gRPC assert path — shipped in #48
updated: 2026-09-10T19:40:00Z
---

# Handoff

The session that wrote this is gone. You are reading the only thing that survived it.
Git history already records *what was done* — this file exists for what git cannot show:
intent, dead ends, and the next move.

## Current item

None in flight. The last session took *Tier validation on the gRPC assert path* from
Claims core and shipped it as PR #48, branch `feat/grpc-tier-validation`, commit c96ea46.
The roadmap slice is marked *shipped (#48)*.

#48 has landed. CI was green and it merged as 0a1254d; the branch is deleted and spent.
Nothing is waiting on you from it.

## State

`main` at 0a1254d, clean, nothing in flight. The next session starts fresh on a slice of
its own choosing.

## What #48 actually did

- `boswell-grpc` now depends on `boswell-gatekeeper`, which it never did before. That
  dependency is the whole point of the slice: the tier/confidence floor lives in one place.
- `Gatekeeper::check_tier_confidence` is public. It is the one rule of `validate` a
  transport can apply alone, because it needs no store. It honors
  `validate_tier_appropriateness`, and `validate` now routes through it rather than
  duplicating the flag check.
- Enforced on `Assert` **and** `Learn`. The roadmap slice named only Assert; Learn is the
  same hole in batch form and was closed with it. One claim below its floor is rejected,
  its neighbours are not.
- `RejectionReason` derives `thiserror::Error`, so it has a `Display` and the transport
  reports the Gatekeeper's wording rather than its own.
- `BosWellServiceImpl::with_validation_config` turns the floor off.

## Tried and rejected

- **Running the full `Gatekeeper::validate` on the Assert path.** Two rules make it wrong
  there, and both would have surfaced as test failures rather than as design arguments if
  it had shipped. `validate_duplicates` does an exact subject/predicate/object query and
  rejects a match — but `SqliteStore::assert_claim` treats a repeat as *corroboration*, so
  running it would reject precisely the writes the corroboration design exists to reward.
  `validate_entity_format` demands `namespace:value` on subject, predicate and object; the
  wire has always accepted bare subjects (`"Alice"`), so enabling it silently breaks every
  existing caller. Tightening either is a separate, breaking decision.
- **Formatting the rejection message inside `boswell-grpc`.** `check_tier_confidence`
  returns `Option<RejectionReason>`, and matching one variant out of five needs a catch-all
  arm that can never fire. Deriving `Display` on the enum was smaller and put the wording
  where the rule is.

## Next step

Pick a fresh slice from `docs/development/roadmap.md`. Assessed but not taken, in rough
order of ratio:

- **gRPC graceful shutdown** (Transport). `crates/boswell-grpc/src/server.rs` calls
  `.serve(addr)`; it wants `serve_with_shutdown`. The Janitor and Synthesizer already
  handle `ctrl_c` correctly and are the model to copy. Smallest real item in the tree.
- **Push the triple match into SQL** (Procedural memory), marked in place at
  `crates/boswell-store/src/procedure_store.rs:351`. Self-contained.
- **Delete the `auth_token` plumbing and enforce the loopback bind** (ADR-021). Decided,
  not researched — fourteen `is_empty()` call sites in `boswell-grpc`, plus the router's
  unread JWT and the SDK carrying it. Bigger than one session may hold; it spans grpc,
  router, sdk and server config. Note the `auth_token.is_empty()` checks are still in the
  code #48 touched, deliberately untouched — that deletion is its own slice.
- **MCP tool surface for goals and procedures** (Transport). The largest feature lag: five
  claim-only tools against fifteen gateway routes. Not sized.

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

## Open questions

None. Nothing here needs a decision before work can start.

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
