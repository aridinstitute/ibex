# SurrealDB 4.0.0 benchmark comparison

## Latest result: adapter-maintained sort keys

The adapter now stores the primary-creator sort key with the original resource content write. Creator order and credited-name changes recompute it there; person renames/deletions refresh distinct affected resources in their transaction. The production schema has neither a per-edge/person refresh event nor a query-valued sort field. Store APIs and logging boundaries are unchanged.

Seed-only results with the current allocator-enabled build and normal release profile:

| Resources | Initial 4.0 seed | Fixed seed |
| --- | ---: | ---: |
| 1,000 | 7.232s (later repeat: 6.737s) | 0.248s |
| 5,000 | 96.948s | 1.131s |

The subsequent filter suite seeded in 0.244s/1.129s at 1K/5K, confirming the smoke-test result. No failed filter workloads or timeouts occurred.

Only resource filtering and SurrealDB-backed person benchmarks were rerun. Mock-store/import controls were not rerun. The current Cargo manifest/lockfile includes the user's allocator/default-feature/dependency changes; these rows are an end-to-end comparison, not a perfectly isolated measurement of the adapter patch alone. No LTO or release-profile changes were made.

At 1K persons versus the original 3.3.4 baseline, create is 51% faster and duplicate-name lookup is 70% faster. Get remains 8% slower, delete 13% slower, and update 91% slower. Thus the extreme seed regression is resolved, but this is not an across-the-board performance win.

Mean-time estimates (positive change means slower):

| Benchmark | 3.3.4 ms | Initial 4.0 ms | Fixed 4.0 ms | Fixed vs 3.3.4 |
| --- | ---: | ---: | ---: | ---: |
| person_crud/surreal_mem_create/1 | 3.5825 | 11.8978 | 7.2615 | +102.7% |
| person_crud/surreal_mem_create/100 | 12.0051 | 70.3258 | 11.2760 | -6.1% |
| person_crud/surreal_mem_create/1000 | 83.6006 | 587.3829 | 40.9103 | -51.1% |
| person_crud/surreal_mem_delete/1 | 0.1297 | 0.9397 | 0.6845 | +427.9% |
| person_crud/surreal_mem_delete/100 | 8.8413 | 61.3508 | 11.8195 | +33.7% |
| person_crud/surreal_mem_delete/1000 | 88.7589 | 605.2805 | 100.0713 | +12.7% |
| person_crud/surreal_mem_get/1 | 0.0129 | 0.0168 | 0.0140 | +8.8% |
| person_crud/surreal_mem_get/100 | 1.3301 | 1.6877 | 1.4078 | +5.8% |
| person_crud/surreal_mem_get/1000 | 13.0949 | 16.6970 | 14.2065 | +8.5% |
| person_crud/surreal_mem_update/1 | 0.0410 | 0.6119 | 0.0594 | +44.8% |
| person_crud/surreal_mem_update/100 | 4.5199 | 63.4476 | 6.7663 | +49.7% |
| person_crud/surreal_mem_update/1000 | 67.0317 | 599.2669 | 128.3207 | +91.4% |
| person_duplicate_find/surreal_mem_find_1k | 709.2010 | 241.3613 | 210.7972 | -70.3% |
| resource_filter_1000/combined/1000 | 5.1231 | 4.3455 | 4.2368 | -17.3% |
| resource_filter_1000/creator/1000 | 1.7113 | 3.0317 | 2.9852 | +74.4% |
| resource_filter_1000/published_range/1000 | 3.8737 | 3.0598 | 3.1899 | -17.7% |
| resource_filter_1000/title_broad/1000 | 1.4812 | 2.5830 | 2.7172 | +83.4% |
| resource_filter_1000/title_filter_title_sort/1000 | 4.3751 | 3.9248 | 3.5918 | -17.9% |
| resource_filter_1000/title_miss/1000 | 3.2909 | 2.5483 | 2.3206 | -29.5% |
| resource_filter_1000/title_narrow/1000 | 3.2312 | 2.9674 | 2.9125 | -9.9% |
| resource_filter_1000/unfiltered/1000 | 1.4692 | 2.5337 | 2.4222 | +64.9% |
| resource_filter_5000/combined/5000 | 9.2772 | 5.0910 | 5.3878 | -41.9% |
| resource_filter_5000/creator/5000 | 5.8085 | 8.9552 | 9.4733 | +63.1% |
| resource_filter_5000/published_range/5000 | 7.8849 | 3.5949 | 4.0022 | -49.2% |
| resource_filter_5000/title_broad/5000 | 5.1802 | 3.1576 | 3.4397 | -33.6% |
| resource_filter_5000/title_filter_title_sort/5000 | 20.3641 | 11.4292 | 12.2084 | -40.0% |
| resource_filter_5000/title_miss/5000 | 16.5942 | 11.9547 | 11.1878 | -32.6% |
| resource_filter_5000/title_narrow/5000 | 7.1070 | 3.4707 | 3.7670 | -47.0% |
| resource_filter_5000/unfiltered/5000 | 5.1365 | 3.0373 | 3.2578 | -36.6% |

Saved Criterion baseline: `surrealdb-4.0.0-adapter-sort-key`.

Validation: all 74 active adapter unit/integration tests passed across the full suite and the added import/rename tests; workspace all-target checks passed. Coverage includes import of new people, rename fanout across primary/secondary/credited creators and multiple roles, clearing/reordering creators, unchanged edit timestamps during sort maintenance, and rollback of both creators and stored keys after a failed replacement. A schema regression test prohibits restoring either the events or the query-valued field. The legacy schema survives only as a diagnostic fixture for the ignored seed probe and standalone write probe.

Earlier measurements and diagnosis below are historical; their shipment recommendation predates this fix.

## Follow-up: isolated write-path diagnosis

On the current allocator-enabled 4.0 build, the same 1K-resource seed took **6.840s with sort-key events enabled and 1.040s with both events removed**, using disposable Mem databases. This is an isolation experiment, not a production fix: removing events leaves creator sort keys stale.

A 3-second macOS `sample` of the existing release benchmark found 1,426 of 1,621 main-thread samples under `relate_creators_in_txn`, including substantial statement/stage planning and compilation. The sample is a window during seeding, not a whole-run CPU percentage.

A minimal 200-edge reproduction took 14.5ms without the sort-key schema, 377.6ms with it, and 9.6ms with the sort field retained but its refresh events removed. Keeping the events while removing the field still took 216.8ms. This implicates event-triggered resource updates and their planning overhead, not only the field's lookup or allocator choice.

The relevant schema is `crates/adapter-surreal/src/primary_creator_sort.surql`: every creator-edge mutation triggers a resource update; each resource update recomputes `primary_creator_last_name` using a query over creator edges. Person-name writes can trigger updates to all authored resources too. This amplifies a logical write into multiple nested database operations. These measurements identify a major cost, but do not prove why all remaining 4.0 overhead occurs or how much of the 5K/97s case it explains.

No production trigger behavior was changed. Reproduce the isolated seed check without Criterion or a dependency rebuild:

```sh
IBEX_BENCH_SIZES=1000 cargo test --release --locked -p ibex-adapter-surreal \
  --lib seed_sort_event_probe -- --ignored --nocapture
cargo run --release --locked -p ibex-adapter-surreal --example write_probe -- 200
```

## Setup

- Baseline: attachment commit `11b62d9`, SDK pinned to 3.3.4; the existing lockfile resolves its engine crates to 3.3.5.
- Candidate: the same application and benchmark code, SDK and engine crates updated to 4.0.0.
- Machine: Apple M4 Pro, 48 GiB RAM, macOS 27.0.1; rustc 1.99.0.
- Release builds, default Cargo bench profile. Baseline ran first, then candidate, sequentially on the same machine.
- Existing Criterion suites were unchanged. Resource filtering used 1,000 and 5,000 seeded resources, 50-row pages, strict error checking, and a 15-second query timeout.
- Person CRUD uses 1, 100, and 1,000 operations. Create includes opening/schema setup; delete excludes setup.
- Import benchmarks use their default sizes (50 and 200); `IBEX_BENCH_SIZES` only controls the resource-filter suite.
- Adapter benchmarks here use SurrealDB Mem. The `memory` person cases and all import cases use the application's mock MemoryStore, not SurrealDB.
- Fresh in-memory databases and temporary directories only; no real library opened.

Values below are Criterion **mean** estimates, not the slope estimates printed as the headline timing in some console output. Positive change means slower. This is one local before/after run, not a controlled multi-run regression verdict. No queries failed or timed out.

## Findings

4.0 fixes the filtered-ordering behavior covered by five previously ignored adapter tests; all five pass in debug and release and are now enabled.

Performance is mixed, with substantial write regressions:
- At 5K resources, seven of eight filter workloads improved by 28–54%; creator filtering regressed 54%.
- At 1K, unfiltered/broad-title/creator queries regressed 72–77%; the other five improved 8–23%.
- Duplicate-name lookup improved 66% (about 2.9× faster).
- At 1K persons, create/update/delete batches are 7.0×/8.9×/6.8× slower; get is 28% slower.
- Single-person update is 14.9× slower. The 1-person create case, which includes open/schema setup, is 3.3× slower.
- Resource seed wall time rose from 0.544s to 7.232s at 1K (13.3×) and 2.686s to 96.948s at 5K (36.1×). These are one-shot seed measurements, not Criterion samples.
- Mock-store/import controls do not show changes remotely large enough to explain the write regressions.

**Recommendation:** investigate the write and seeding regressions before treating this upgrade as ready to ship. These results do not establish the upstream root cause or disk-backed production performance. No query rewrites or tuning were added to mask the version comparison. A 100K resource run was not attempted.

## Resource filters

| Benchmark | 3.3.4 (ms) | 4.0.0 (ms) | Change |
| --- | ---: | ---: | ---: |
| resource_filter_1000/combined/1000 | 5.1231 | 4.3455 | -15.2% |
| resource_filter_1000/creator/1000 | 1.7113 | 3.0317 | +77.2% |
| resource_filter_1000/published_range/1000 | 3.8737 | 3.0598 | -21.0% |
| resource_filter_1000/title_broad/1000 | 1.4812 | 2.5830 | +74.4% |
| resource_filter_1000/title_filter_title_sort/1000 | 4.3751 | 3.9248 | -10.3% |
| resource_filter_1000/title_miss/1000 | 3.2909 | 2.5483 | -22.6% |
| resource_filter_1000/title_narrow/1000 | 3.2312 | 2.9674 | -8.2% |
| resource_filter_1000/unfiltered/1000 | 1.4692 | 2.5337 | +72.5% |
| resource_filter_5000/combined/5000 | 9.2772 | 5.0910 | -45.1% |
| resource_filter_5000/creator/5000 | 5.8085 | 8.9552 | +54.2% |
| resource_filter_5000/published_range/5000 | 7.8849 | 3.5949 | -54.4% |
| resource_filter_5000/title_broad/5000 | 5.1802 | 3.1576 | -39.0% |
| resource_filter_5000/title_filter_title_sort/5000 | 20.3641 | 11.4292 | -43.9% |
| resource_filter_5000/title_miss/5000 | 16.5942 | 11.9547 | -28.0% |
| resource_filter_5000/title_narrow/5000 | 7.1070 | 3.4707 | -51.2% |
| resource_filter_5000/unfiltered/5000 | 5.1365 | 3.0373 | -40.9% |

## SurrealDB person operations

| Benchmark | 3.3.4 (ms) | 4.0.0 (ms) | Change |
| --- | ---: | ---: | ---: |
| person_crud/surreal_mem_create/1 | 3.5825 | 11.8978 | +232.1% |
| person_crud/surreal_mem_create/100 | 12.0051 | 70.3258 | +485.8% |
| person_crud/surreal_mem_create/1000 | 83.6006 | 587.3829 | +602.6% |
| person_crud/surreal_mem_delete/1 | 0.1297 | 0.9397 | +624.7% |
| person_crud/surreal_mem_delete/100 | 8.8413 | 61.3508 | +593.9% |
| person_crud/surreal_mem_delete/1000 | 88.7589 | 605.2805 | +581.9% |
| person_crud/surreal_mem_get/1 | 0.0129 | 0.0168 | +30.2% |
| person_crud/surreal_mem_get/100 | 1.3301 | 1.6877 | +26.9% |
| person_crud/surreal_mem_get/1000 | 13.0949 | 16.6970 | +27.5% |
| person_crud/surreal_mem_update/1 | 0.0410 | 0.6119 | +1392.4% |
| person_crud/surreal_mem_update/100 | 4.5199 | 63.4476 | +1303.7% |
| person_crud/surreal_mem_update/1000 | 67.0317 | 599.2669 | +794.0% |
| person_duplicate_find/surreal_mem_find_1k | 709.2010 | 241.3613 | -66.0% |

## Controls (no SurrealDB)

| Benchmark | 3.3.4 (ms) | 4.0.0 (ms) | Change |
| --- | ---: | ---: | ---: |
| import_parse/biblatex/200 | 1.6617 | 1.6027 | -3.6% |
| import_parse/biblatex/50 | 0.4051 | 0.3926 | -3.1% |
| import_parse/csl_json/200 | 0.8269 | 0.8212 | -0.7% |
| import_parse/csl_json/50 | 0.2019 | 0.1984 | -1.7% |
| import_save/biblatex_conflicting_metadata_200 | 1.5426 | 1.4851 | -3.7% |
| import_save/biblatex_conflicting_metadata_50 | 0.3627 | 0.3653 | +0.7% |
| import_save/biblatex_shared_creators_containers_200 | 2.6069 | 2.5811 | -1.0% |
| import_save/biblatex_shared_creators_containers_50 | 0.6334 | 0.6341 | +0.1% |
| import_save/biblatex_unique_records_200 | 2.8220 | 2.7593 | -2.2% |
| import_save/biblatex_unique_records_50 | 0.6749 | 0.6759 | +0.1% |
| import_save/csl_json_conflicting_metadata_200 | 1.0571 | 1.0476 | -0.9% |
| import_save/csl_json_conflicting_metadata_50 | 0.2690 | 0.2613 | -2.9% |
| import_save/csl_json_shared_creators_containers_200 | 1.8541 | 1.7970 | -3.1% |
| import_save/csl_json_shared_creators_containers_50 | 0.4415 | 0.4412 | -0.1% |
| import_save/csl_json_unique_records_200 | 1.9947 | 1.9443 | -2.5% |
| import_save/csl_json_unique_records_50 | 0.4936 | 0.4731 | -4.2% |
| person_crud/memory_create/1 | 0.0008 | 0.0008 | +4.3% |
| person_crud/memory_create/100 | 0.0525 | 0.0544 | +3.6% |
| person_crud/memory_create/1000 | 0.5912 | 0.6231 | +5.4% |
| person_crud/memory_delete/1 | 0.0006 | 0.0006 | -0.4% |
| person_crud/memory_delete/100 | 0.0382 | 0.0371 | -3.0% |
| person_crud/memory_delete/1000 | 0.4199 | 0.4363 | +3.9% |
| person_crud/memory_get/1 | 0.0001 | 0.0001 | +1.6% |
| person_crud/memory_get/100 | 0.0061 | 0.0061 | +0.6% |
| person_crud/memory_get/1000 | 0.0634 | 0.0649 | +2.2% |
| person_crud/memory_update/1 | 0.0010 | 0.0011 | +0.9% |
| person_crud/memory_update/100 | 0.1020 | 0.1044 | +2.3% |
| person_crud/memory_update/1000 | 1.0312 | 1.0489 | +1.7% |
| person_duplicate_find/memory_find_1k | 0.3773 | 0.3706 | -1.8% |

## Reproduce

On the baseline revision, run each command with `VERSION=3.3.4`; on the upgraded revision use `VERSION=4.0.0`:

```sh
IBEX_BENCH_SIZES=1000,5000 IBEX_BENCH_STRICT=1 \
  cargo bench --locked -p ibex-adapter-surreal --bench resource_filter \
  -- --save-baseline surrealdb-$VERSION
cargo bench --locked -p ibex-adapter-surreal --bench person_crud \
  -- --save-baseline surrealdb-$VERSION
cargo bench --locked -p ibex-app --features testing --bench import_pipeline \
  -- --save-baseline surrealdb-$VERSION
```

Named estimates are retained locally under `target/criterion/**/surrealdb-{3.3.4,4.0.0}/estimates.json`. Do not compare older `base`, `new`, or `change` directories left by unrelated runs.

## Validation and debug stack requirement

- Core/app/adapter tests pass after the upgrade; all 50 adapter unit tests pass, including the five re-enabled ordering tests.
- Release adapter unit tests also pass without a stack override.
- `cargo check --workspace --all-targets` passed.
- A full workspace test attempt exceeded 20 minutes; full CLI/GUI test completion is not claimed.
- Unoptimized 4.0 queries overflowed the default test-thread stack on this macOS machine; a 16 MiB stack allowed all core/app/adapter tests to pass. `.cargo/config.toml` sets `RUST_MIN_STACK=16777216` for Cargo-launched processes (without overriding an explicitly set environment value). This is a local-development/test setting, not an installed application's runtime change.
- 3.x and 4.0 library files must not be opened interchangeably. No data migration was performed.
