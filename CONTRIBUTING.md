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

## Architecture at a glance

Boswell follows Clean Architecture: `boswell-domain` has zero external
dependencies; application, infrastructure, and interface layers depend inward.
When adding behavior, put domain logic in `boswell-domain` and keep I/O at the
edges. Significant technical decisions are recorded as ADRs — add one when you
make a decision worth remembering.

## Licensing of contributions

Boswell is licensed under the GNU AGPL-3.0 (see [LICENSE](LICENSE)). By
submitting a contribution, you agree that it is licensed under the same terms.
