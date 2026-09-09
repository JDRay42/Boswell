# Contributing to Boswell

Thanks for your interest in Boswell. This file covers the essentials; see
[docs/architecture/14-contributing.md](docs/architecture/14-contributing.md) for
the deeper architectural conventions and
[docs/ADRs/](docs/ADRs/) for the decisions behind the design.

## Ground rules

- Be respectful. This project follows a [Code of Conduct](CODE_OF_CONDUCT.md).
- Discuss non-trivial changes in an issue before opening a large PR.

## Before you open a PR

Every change must pass the same bar CI enforces:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

CI runs these on every push and pull request. Warnings are treated as errors, so
keep the tree clean.

Notes:
- Tests that require a live [Ollama](https://ollama.com/) server (real embedding
  or LLM calls) are marked `#[ignore]` and are skipped by default. Run them
  locally with `cargo test -- --ignored` when relevant.
- Building the gRPC crate needs the protobuf compiler (`brew install protobuf`
  or `apt-get install protobuf-compiler`).

## Documentation rules

Two rules that exist because breaking them has already cost this project real time.

**The roadmap ships with the change.** [`docs/development/roadmap.md`](docs/development/roadmap.md)
is the single source of truth for where Boswell stands, and it is updated in the *same* PR
as the work it describes. A feature PR that leaves it untouched is incomplete. Status
recorded separately, after the fact, by whoever remembers, does not survive contact with a
real week — the previous roadmap had zero of thirty-six boxes checked while two phases were
essentially complete.

**Where the docs specify behaviour, the code changes, not the docs.** When documentation
states how something works and the code disagrees, the default is that the code is wrong.
`boswell learn` takes its file positionally because `docs/importing-personal-memory.md` says
so; the parser was fixed to match, and the documented form must keep parsing. Reversing that
— editing the doc to match whatever the code happens to do — silently breaks every reader
who followed it.

## Architecture at a glance

Boswell follows Clean Architecture: `boswell-domain` depends on nothing but `uuid`
(for UUIDv7 identifiers, per ADR-011); application, infrastructure, and interface
layers depend inward.
When adding behavior, put domain logic in `boswell-domain` and keep I/O at the
edges. Significant technical decisions are recorded as ADRs — add one when you
make a decision worth remembering.

## Licensing of contributions

Boswell is licensed under the GNU AGPL-3.0 (see [LICENSE](LICENSE)). By
submitting a contribution, you agree that it is licensed under the same terms.
