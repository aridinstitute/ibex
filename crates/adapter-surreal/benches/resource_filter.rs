//! Resource filtering benchmarks against the SurrealDB in-memory adapter,
//! the backend the GUI runs on.
//!
//! Every case funnels through the one filter seam,
//! [`Store::list_resources_query_page`], so the numbers describe the same
//! query path the library view uses: filter predicates AND together ahead
//! of the ordering, then a `limit`/`offset` window.
//!
//! Library sizes come from `IBEX_BENCH_SIZES` (comma-separated, default
//! `1000`) so a 100k run is opt-in rather than a surprise in plain
//! `cargo bench`:
//!
//! ```text
//! cargo bench -p ibex-adapter-surreal --bench resource_filter
//! IBEX_BENCH_SIZES=1000,5000,100000 cargo bench -p ibex-adapter-surreal --bench resource_filter
//! ```
//!
//! Seeding is measured once per size with a wall clock and then reused by
//! every query case: 100k `create_resource` calls inside criterion's
//! iteration loop would multiply a multi-minute seed by the sample count.
//!
//! Every query runs with SurrealDB's own 15s query timeout. The `creator`
//! predicate is a correlated subquery that grows with resources × edges, so
//! at large sizes it is expected to hit the cap; a timed-out case is
//! measured as a timeout instead of aborting the run or hanging for hours.

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use ibex_adapter_surreal::SurrealStore;
use ibex_app::seed::seed_demo_library;
use ibex_core::{
    NonEmptyStr, PersonId, PublishedRange, ResourceFilter, ResourceQuery, ResourceSort,
    SortDirection, Store, UtcMillis,
};
use std::hint::black_box;
use std::time::{Duration, Instant};
use tokio::runtime::Runtime;

/// One library page, the same size the GUI requests.
const PAGE_LIMIT: u32 = 50;

/// SurrealDB's per-query timeout for every benchmark case.
const QUERY_TIMEOUT: Duration = Duration::from_secs(15);

/// Sizes to seed and benchmark when `IBEX_BENCH_SIZES` is unset.
const DEFAULT_SIZES: &[usize] = &[1_000];

/// Case whose needle matches nothing, so the empty-page guard skips it.
const MISS_CASE: &str = "title_miss";

fn sizes() -> Vec<usize> {
    match std::env::var("IBEX_BENCH_SIZES") {
        Ok(raw) => raw
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.parse().expect("IBEX_BENCH_SIZES entries are counts"))
            .collect(),
        Err(_) => DEFAULT_SIZES.to_vec(),
    }
}

fn needle(s: &str) -> NonEmptyStr {
    NonEmptyStr::try_new(s).expect("bench needles are non-empty")
}

fn year(y: i64) -> UtcMillis {
    UtcMillis::jan_1_utc(y).expect("bench years are civil")
}

/// The filter shapes the library view can send, from no predicate at all to
/// every predicate ANDed together, plus a miss that must scan the library
/// without matching.
///
/// Selectivity against the demo seed (8 kinds, titles cycling by kind):
/// - `title_narrow` matches one kind in eight;
/// - `title_broad` matches nearly every row, the expensive end;
/// - `creator` walks one person's `created` edges, so every person matches;
/// - `published_range` brackets dated rows across a decade;
/// - `combined` ANDs a title and a window;
/// - `title_miss` matches nothing and never short-circuits.
fn cases(creator: PersonId) -> Vec<(&'static str, ResourceQuery)> {
    let title_only = |needle_text: &str| ResourceQuery {
        sort: None,
        filter: ResourceFilter {
            search: None,
            title: Some(needle(needle_text)),
            ..ResourceFilter::none()
        },
    };
    vec![
        ("unfiltered", ResourceQuery::newest_first()),
        ("title_narrow", title_only("Citation")),
        ("title_broad", title_only("e")),
        (
            "creator",
            ResourceQuery {
                sort: None,
                filter: ResourceFilter {
                    search: None,
                    creator: Some(creator),
                    ..ResourceFilter::none()
                },
            },
        ),
        (
            "published_range",
            ResourceQuery {
                sort: None,
                filter: ResourceFilter {
                    search: None,
                    published: Some(PublishedRange {
                        from: Some(year(1970)),
                        to: Some(year(1980)),
                    }),
                    ..ResourceFilter::none()
                },
            },
        ),
        (
            "combined",
            ResourceQuery {
                sort: None,
                filter: ResourceFilter {
                    search: None,
                    title: Some(needle("Citation")),
                    published: Some(PublishedRange {
                        from: Some(year(1970)),
                        to: Some(year(2000)),
                    }),
                    ..ResourceFilter::none()
                },
            },
        ),
        (MISS_CASE, title_only("zzz-no-such-title")),
        (
            "title_filter_title_sort",
            ResourceQuery {
                sort: Some((ResourceSort::Title, SortDirection::Ascending)),
                filter: ResourceFilter {
                    search: None,
                    title: Some(needle("Report")),
                    ..ResourceFilter::none()
                },
            },
        ),
    ]
}

fn bench_resource_filter(c: &mut Criterion) {
    let rt = Runtime::new().expect("runtime");
    let mut failed = false;

    for n in sizes() {
        let store = rt
            .block_on(SurrealStore::open_memory_with_query_timeout(Some(
                QUERY_TIMEOUT,
            )))
            .expect("open");
        let seeded = Instant::now();
        rt.block_on(seed_demo_library(&store, n)).expect("seed");
        let seed_elapsed = seeded.elapsed();
        let rows_per_sec = n as f64 / seed_elapsed.as_secs_f64();
        // Wall-clock readout next to the criterion report: the question a
        // first large run asks is how long seeding costs before any query.
        eprintln!(
            "resource_filter: seeded {n} rows in {seed_elapsed:?} ({rows_per_sec:.0} rows/s)"
        );

        // Any person authors seeded rows, so the creator case never
        // measures an empty edge walk.
        let creator = rt
            .block_on(store.list_persons())
            .expect("list persons")
            .first()
            .expect("seed creates people")
            .id;

        let cases = cases(creator);
        let mut group = c.benchmark_group(format!("resource_filter/{n}"));
        group.throughput(Throughput::Elements(n as u64));
        // Queries are cheap next to the seed, so keep each case short.
        group.sample_size(20);
        group.warm_up_time(Duration::from_secs(1));
        group.measurement_time(Duration::from_secs(3));

        for (name, query) in &cases {
            // A case that silently matches nothing would benchmark the
            // planner's empty path, not the filter under test. A case that
            // times out is reported and still measured below.
            match rt.block_on(store.list_resources_query_page(query, 0, PAGE_LIMIT)) {
                Ok(page) if *name != MISS_CASE => assert!(
                    !page.rows.is_empty(),
                    "case {name} matched no rows at n={n}; seed or filter drifted"
                ),
                Ok(_) => {}
                Err(err) => {
                    failed = true;
                    eprintln!("resource_filter: case {name} at n={n} failed its guard query: {err}")
                }
            }

            group.bench_with_input(BenchmarkId::new(*name, n), &n, |b, _| {
                b.iter(|| {
                    // Timeouts are an outcome here, not a failure: aborting
                    // the run would discard every healthy case after them.
                    let page = rt.block_on(store.list_resources_query_page(
                        black_box(query),
                        0,
                        PAGE_LIMIT,
                    ));
                    failed |= page.is_err();
                    let _ = black_box(page);
                });
            });
        }

        group.finish();
    }

    // Keep local exploratory runs useful, but never publish CI timings for
    // failed queries as if they represented successful work.
    if std::env::var_os("IBEX_BENCH_STRICT").is_some() {
        assert!(!failed, "resource_filter: one or more workloads failed");
    }
}

criterion_group!(benches, bench_resource_filter);
criterion_main!(benches);
