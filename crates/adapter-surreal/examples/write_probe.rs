//! Small write-path diagnostic, not a replacement for application benchmarks.
//! cargo run --release --locked -p ibex-adapter-surreal --example write_probe -- 200
use std::time::Instant;

use surrealdb::{Surreal, engine::local::Mem, types::RecordId};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let n: usize = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "200".into())
        .parse()?;
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(10 * 1024 * 1024)
        .build()?;
    rt.block_on(async {
        for mode in ["parameter", "sort_events", "sort_field_only", "sort_events_no_field", "transaction", "single_transaction", "insert", "no_indexes", "unenforced"] {
            let db = Surreal::new::<Mem>(()).await?;
            db.use_ns("probe").use_db("probe").await?;
            let enforcement = if mode == "unenforced" { "" } else { "ENFORCED" };
            db.query(format!(
                "DEFINE TABLE person SCHEMALESS; DEFINE TABLE resource SCHEMALESS; \
                 DEFINE TABLE created TYPE RELATION IN person OUT resource {enforcement}; \
                 CREATE person:author; CREATE resource:work;"
            )).await?.check()?;
            if mode != "no_indexes" {
                db.query("DEFINE INDEX created_in ON created FIELDS in; DEFINE INDEX created_out ON created FIELDS out;")
                    .await?.check()?;
            }
            if mode.starts_with("sort_") {
                db.query(include_str!("../tests/fixtures/legacy_creator_sort.surql")).await?.check()?;
                if mode == "sort_field_only" {
                    db.query("REMOVE EVENT refresh_primary_creator_on_edge ON created; REMOVE EVENT refresh_primary_creator_on_name ON person;").await?.check()?;
                } else if mode == "sort_events_no_field" {
                    db.query("REMOVE FIELD primary_creator_last_name ON resource").await?.check()?;
                }
            }
            let from = RecordId::new("person", "author");
            let to = RecordId::new("resource", "work");
            let sql = if mode == "insert" {
                "INSERT RELATION INTO created { in: $from, out: $to, role: 'author', order: 0, credited_as: NONE }"
            } else {
                "RELATE $from -> created -> $to SET role = 'author', order = 0, credited_as = NONE"
            };
            let started = Instant::now();
            if mode == "single_transaction" {
                let txn = db.clone().begin().await?;
                for _ in 0..n {
                    txn.query(sql).bind(("from", from.clone())).bind(("to", to.clone())).await?.check()?;
                }
                txn.commit().await?;
            } else {
                for _ in 0..n {
                    if mode == "transaction" {
                        let txn = db.clone().begin().await?;
                        txn.query(sql).bind(("from", from.clone())).bind(("to", to.clone())).await?.check()?;
                        txn.commit().await?;
                    } else {
                        db.query(sql).bind(("from", from.clone())).bind(("to", to.clone())).await?.check()?;
                    }
                }
            }
            let elapsed = started.elapsed();
            let count: Option<i64> = db.query("SELECT VALUE count() FROM created GROUP ALL").await?.take(0)?;
            assert_eq!(count, Some(n as i64));
            println!("{mode}: {n} relations in {elapsed:?}, {:.3} ms/edge", elapsed.as_secs_f64() * 1000.0 / n as f64);
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    })
}
