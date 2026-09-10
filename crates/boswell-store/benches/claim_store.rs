//! Benchmarks for the claim store's two hot paths: asserting a claim and
//! querying for one.
//!
//! These exist to settle a figure the February plan asserted and nothing ever
//! measured — "100+ assertions/sec, queries <100ms p95". Run them with:
//!
//! ```text
//! cargo bench -p boswell-store
//! ```
//!
//! CI compiles this target (`cargo clippy --workspace --all-targets`) and runs
//! it once per benchmark in test mode (`cargo test --workspace`), but never
//! times it: a shared runner's numbers are noise, and a benchmark that gates a
//! merge on wall-clock is a flaky test wearing a hat.
//!
//! # What is and is not measured
//!
//! - **Asserts run against a file-backed database**, because durability is the
//!   cost. `SqliteStore` sets no journal or synchronous pragma, so SQLite's
//!   defaults apply — `journal_mode=delete`, `synchronous=full` — and every
//!   `assert_claim` is its own rollback journal and its own fsync. That is the
//!   shipped behavior; hiding it behind an in-memory database would measure a
//!   system nobody runs. Note that on macOS a plain `fsync(2)` returns once the
//!   write reaches the drive's cache rather than its platter, so these numbers
//!   are a ceiling for any platform whose fsync is a full barrier.
//! - **Queries run against an in-memory database.** A corpus this size lives
//!   entirely in the page cache after its first read either way, so the file
//!   adds setup cost and no realism.
//! - **Vector search is off throughout.** With it on, `assert_claim` embeds
//!   through `MockEmbeddingModel`, whose cost is a property of the mock rather
//!   than of anything shipped. Benchmarking the real embedder means
//!   benchmarking Ollama over a socket, which belongs in its own target.
//! - **The corpus is generated deterministically** from the row index, so two
//!   runs on the same machine compare.

use boswell_domain::traits::{ClaimQuery, ClaimStore};
use boswell_domain::{Claim, ClaimId};
use boswell_store::SqliteStore;
use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput};
use std::hint::black_box;
use std::path::Path;

/// Corpus size for the "small instance" case.
const SMALL: usize = 1_000;

/// Corpus size for the "1000+ claims without degradation" case the Phase 5
/// validation list names.
const LARGE: usize = 10_000;

/// Claims asserted per timed iteration. One assert is a single fsync and lands
/// well inside criterion's timer resolution; a hundred makes the per-claim rate
/// readable straight off `Throughput::Elements`.
const ASSERT_BATCH: usize = 100;

/// The tiers a generated claim cycles through, in `Tier::as_str` spelling.
const TIERS: [&str; 4] = ["ephemeral", "task", "project", "permanent"];

/// Build the `n`th claim of a generated corpus.
///
/// The moduli are coprime-ish with each other and with the corpus sizes above so
/// that a filter on any one field selects a spread of rows rather than a
/// contiguous block. `subject` is the most selective (about two rows per value
/// at `SMALL`), `tier` the least (a quarter of the corpus).
fn generated_claim(n: usize) -> Claim {
    Claim::new(
        ClaimId::from_value(n as u128 + 1),
        "bench".to_string(),
        format!("entity:{}", n % 512),
        format!("relates_to_{}", n % 7),
        format!("value:{}", n % 1024),
        {
            let lower = 0.5 + (n % 25) as f64 / 100.0;
            (lower, lower + 0.2)
        },
        TIERS[n % TIERS.len()].to_string(),
        1_700_000_000 + n as u64,
    )
}

/// A store with `n` generated claims in it, at `path` (`:memory:` for in-memory).
fn corpus(path: impl AsRef<Path>, n: usize) -> SqliteStore {
    let mut store = SqliteStore::new(path, false, 0).expect("open bench store");
    for i in 0..n {
        store.assert_claim(generated_claim(i)).expect("seed claim");
    }
    store
}

/// An otherwise-empty query, so each benchmark below sets exactly the one field
/// it is measuring.
fn empty_query() -> ClaimQuery {
    ClaimQuery {
        namespace: None,
        subject: None,
        predicate: None,
        object: None,
        tier: None,
        source_type: None,
        min_confidence: None,
        semantic_text: None,
        limit: None,
    }
}

/// Assert throughput against a fresh file-backed database per iteration.
///
/// The store is created in setup and dropped after timing, so neither schema
/// initialization nor the connection close is counted — only the asserts.
fn assert_throughput(c: &mut Criterion) {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut run = 0usize;

    let mut group = c.benchmark_group("assert");
    group.throughput(Throughput::Elements(ASSERT_BATCH as u64));
    group.bench_function("assert_claim/fsync-per-claim", |b| {
        b.iter_batched(
            || {
                run += 1;
                let path = dir.path().join(format!("assert-{run}.db"));
                SqliteStore::new(path, false, 0).expect("open bench store")
            },
            |mut store| {
                for i in 0..ASSERT_BATCH {
                    black_box(store.assert_claim(generated_claim(i)).expect("assert"));
                }
                store
            },
            BatchSize::PerIteration,
        );
    });
    group.finish();
}

/// Point and filtered reads over a corpus of `n` claims.
fn query_latency(c: &mut Criterion, store: &SqliteStore, n: usize) {
    let mut group = c.benchmark_group("query");

    group.bench_with_input(BenchmarkId::new("get_claim", n), &n, |b, &n| {
        // Walk the id space rather than re-reading one row, so the measurement
        // is a lookup and not a page-cache hit on the same page every time.
        let mut next = 0usize;
        b.iter(|| {
            next = (next + 1) % n;
            black_box(store.get_claim(ClaimId::from_value(next as u128 + 1))).expect("get")
        });
    });

    group.bench_with_input(BenchmarkId::new("query_claims/subject", n), &n, |b, _| {
        let query = ClaimQuery {
            subject: Some("entity:17".to_string()),
            ..empty_query()
        };
        b.iter(|| black_box(store.query_claims(&query)).expect("query"));
    });

    group.bench_with_input(BenchmarkId::new("query_claims/tier", n), &n, |b, _| {
        // A quarter of the corpus comes back, so this is dominated by decoding
        // rows rather than by finding them.
        let query = ClaimQuery {
            tier: Some("project".to_string()),
            ..empty_query()
        };
        b.iter(|| black_box(store.query_claims(&query)).expect("query"));
    });

    group.bench_with_input(BenchmarkId::new("query_claims/limit-100", n), &n, |b, _| {
        // The shape a paged read takes: no filter, bounded result.
        let query = ClaimQuery {
            limit: Some(100),
            ..empty_query()
        };
        b.iter(|| black_box(store.query_claims(&query)).expect("query"));
    });

    group.finish();
}

/// `count_claims` against the query-then-length it replaced.
///
/// The two are contractually equivalent (see `ClaimStore::count_claims`), so
/// this is the whole justification for `SqliteStore`'s override in one pair of
/// numbers. `query_claims(..).len()` here *is* the trait's default
/// implementation, spelled out rather than called, because `SqliteStore`
/// overrides it.
fn counting(c: &mut Criterion, store: &SqliteStore, n: usize) {
    let query = empty_query();

    let mut group = c.benchmark_group("count");
    group.throughput(Throughput::Elements(n as u64));

    group.bench_with_input(BenchmarkId::new("count_claims", n), &n, |b, _| {
        b.iter(|| black_box(store.count_claims(&query)).expect("count"));
    });

    group.bench_with_input(BenchmarkId::new("query_claims_len", n), &n, |b, _| {
        b.iter(|| black_box(store.query_claims(&query).expect("query").len()));
    });

    group.finish();
}

fn benches(c: &mut Criterion) {
    assert_throughput(c);

    let small = corpus(":memory:", SMALL);
    let large = corpus(":memory:", LARGE);

    query_latency(c, &small, SMALL);
    query_latency(c, &large, LARGE);
    counting(c, &large, LARGE);
}

criterion_group!(claim_store, benches);
criterion_main!(claim_store);
