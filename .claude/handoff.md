---
status: continue
item: none — first run, nothing picked up yet
updated: 2026-09-10T00:00:00Z
---

# Handoff

The session that wrote this is gone. You are reading the only thing that survived it.
Git history already records *what was done* — this file exists for what git cannot show:
intent, dead ends, and the next move.

## Current item

None. This file was bootstrapped, not worked. Pick the first item from
`docs/development/roadmap.md` yourself and replace this section with it and its
acceptance criteria.

Note that the roadmap's slices carry no order — the file says so explicitly, because
numbering "implies an order the work does not have". Topmost is therefore not the same as
next. Choose on what is actually unblocked, and record why you chose it.

## State

Clean tree on `main` at c04ebe0. Nothing in flight: no slice in the roadmap carries the
*in flight* status.

## Tried and rejected

Nothing yet.

## Next step

Read `docs/development/roadmap.md`, choose one *open* slice, and work it to a commit.
Two candidates were visible in the Claims core workstream at bootstrap time, both real
gaps rather than aspirations:

- Tier validation on the gRPC assert path. `boswell-grpc` does not depend on
  `boswell-gatekeeper` at all, so a direct `Assert` bypasses validation entirely.
- LLM-backed semantic validation in the Gatekeeper, distinct from the semantic duplicate
  detection that already ships.

Neither was assessed for difficulty. Read the roadmap in full before committing to one —
there are workstreams below Claims core that this bootstrap did not read.

## Environment notes

- Verify with all three, matching CI exactly: `cargo fmt --all -- --check`,
  `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.
  CI builds default features only, so feature-gated code is neither built nor tested.
- Tests needing a live Ollama server are `#[ignore]`d. Run them with
  `cargo test -- --ignored` when the work touches embeddings or LLM calls.
- Building `boswell-grpc` needs `protoc` on PATH.
- `git fetch` before branching. Branching off a stale local `main` has bitten this repo
  before — a just-merged PR was missing and it surfaced as an unrelated missing field.

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
  `backlog-loop.conf`. `settings.local.json` is ignored globally.
