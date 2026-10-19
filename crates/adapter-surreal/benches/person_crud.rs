use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use oeuvre_adapter_surreal::SurrealStore;
use oeuvre_app::testing::MemoryStore;
use oeuvre_core::{Name, PersonId, Store};
use std::hint::black_box;
use tokio::runtime::Runtime;

fn sample_name(i: u64) -> Name {
    Name::try_new(
        Some(format!("First{i}")),
        Some(format!("Last{i}")),
        None::<String>,
    )
    .expect("valid name")
}

fn people_seed(n: usize) -> Vec<Name> {
    (0..n as u64).map(sample_name).collect()
}

async fn seed_store(store: &dyn Store, names: &[Name]) -> Vec<PersonId> {
    let mut ids = Vec::with_capacity(names.len());
    for name in names {
        let person = store
            .create_person(name.clone(), vec![])
            .await
            .expect("create");
        ids.push(person.id);
    }
    ids
}

fn bench_person_crud(c: &mut Criterion) {
    let rt = Runtime::new().expect("runtime");
    let mut group = c.benchmark_group("person_crud");

    for &n in &[1usize, 100, 1_000] {
        group.throughput(Throughput::Elements(n as u64));

        group.bench_with_input(BenchmarkId::new("memory/create", n), &n, |b, &n| {
            b.iter(|| {
                rt.block_on(async {
                    let store = MemoryStore::new();
                    for i in 0..n as u64 {
                        let person = store
                            .create_person(sample_name(i), vec![])
                            .await
                            .expect("create");
                        black_box(person);
                    }
                });
            });
        });

        group.bench_with_input(BenchmarkId::new("surreal_mem/create", n), &n, |b, &n| {
            b.iter(|| {
                rt.block_on(async {
                    let store = SurrealStore::open_memory().await.expect("open");
                    for i in 0..n as u64 {
                        let person = store
                            .create_person(sample_name(i), vec![])
                            .await
                            .expect("create");
                        black_box(person);
                    }
                });
            });
        });

        group.bench_with_input(BenchmarkId::new("memory/get", n), &n, |b, &n| {
            let names = people_seed(n);
            let store = MemoryStore::new();
            let ids = rt.block_on(seed_store(&store, &names));
            b.iter(|| {
                rt.block_on(async {
                    for id in &ids {
                        let person = store.get_person(*id).await.expect("get");
                        black_box(person);
                    }
                });
            });
        });

        group.bench_with_input(BenchmarkId::new("surreal_mem/get", n), &n, |b, &n| {
            let names = people_seed(n);
            let store = rt.block_on(SurrealStore::open_memory()).expect("open");
            let ids = rt.block_on(seed_store(&store, &names));
            b.iter(|| {
                rt.block_on(async {
                    for id in &ids {
                        let person = store.get_person(*id).await.expect("get");
                        black_box(person);
                    }
                });
            });
        });

        group.bench_with_input(BenchmarkId::new("memory/update", n), &n, |b, &n| {
            let names = people_seed(n);
            let store = MemoryStore::new();
            let ids = rt.block_on(seed_store(&store, &names));
            b.iter(|| {
                rt.block_on(async {
                    for (i, id) in ids.iter().enumerate() {
                        let person = store
                            .update_person(*id, sample_name(i as u64 + 10_000), vec![])
                            .await
                            .expect("update");
                        black_box(person);
                    }
                });
            });
        });

        group.bench_with_input(BenchmarkId::new("surreal_mem/update", n), &n, |b, &n| {
            let names = people_seed(n);
            let store = rt.block_on(SurrealStore::open_memory()).expect("open");
            let ids = rt.block_on(seed_store(&store, &names));
            b.iter(|| {
                rt.block_on(async {
                    for (i, id) in ids.iter().enumerate() {
                        let person = store
                            .update_person(*id, sample_name(i as u64 + 10_000), vec![])
                            .await
                            .expect("update");
                        black_box(person);
                    }
                });
            });
        });

        group.bench_with_input(BenchmarkId::new("memory/delete", n), &n, |b, &n| {
            b.iter_batched(
                || {
                    let store = MemoryStore::new();
                    let names = people_seed(n);
                    let ids = rt.block_on(seed_store(&store, &names));
                    (store, ids)
                },
                |(store, ids)| {
                    rt.block_on(async move {
                        for id in ids {
                            store.delete_person(id).await.expect("delete");
                        }
                    });
                },
                criterion::BatchSize::SmallInput,
            );
        });

        group.bench_with_input(BenchmarkId::new("surreal_mem/delete", n), &n, |b, &n| {
            b.iter_batched(
                || {
                    let store = rt.block_on(SurrealStore::open_memory()).expect("open");
                    let names = people_seed(n);
                    let ids = rt.block_on(seed_store(&store, &names));
                    (store, ids)
                },
                |(store, ids)| {
                    rt.block_on(async move {
                        for id in ids {
                            store.delete_person(id).await.expect("delete");
                        }
                    });
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

criterion_group!(benches, bench_person_crud);
criterion_main!(benches);
