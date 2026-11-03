//! Compare the person-to-resource lookup forms used before rename/delete.
//! cargo run --release --locked -p ibex-adapter-surreal --example person_lookup_probe -- 1000
use std::time::Instant;
use surrealdb::{Surreal, engine::local::Mem, types::RecordId};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let n: usize = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "1000".into())
        .parse()?;
    tokio::runtime::Builder::new_multi_thread().enable_all()
        .thread_stack_size(10 * 1024 * 1024).build()?.block_on(async {
            let db = Surreal::new::<Mem>(()).await?;
            db.use_ns("probe").use_db("probe").await?;
            db.query("DEFINE TABLE person SCHEMALESS; DEFINE TABLE resource SCHEMALESS; \
                DEFINE TABLE created TYPE RELATION IN person OUT resource ENFORCED; \
                DEFINE INDEX created_in ON created FIELDS in; CREATE person:one; CREATE resource:one;")
                .await?.check()?;
            for edges in [false, true] {
                if edges {
                    db.query("RELATE person:one->created->resource:one; RELATE person:one->created->resource:one;")
                        .await?.check()?;
                }
                for (label, sql) in [
                    ("group", "SELECT VALUE out FROM created WHERE in = $person GROUP BY out"),
                    ("plain", "SELECT VALUE out FROM created WHERE in = $person"),
                    ("graph", "SELECT VALUE out FROM $person->created"),
                ] {
                    let started = Instant::now();
                    for _ in 0..n {
                        let txn = db.clone().begin().await?;
                        let mut response = txn.query(sql).bind(("person", RecordId::new("person", "one"))).await?;
                        let ids: Vec<RecordId> = response.take(0)?;
                        let unique: std::collections::HashSet<_> = ids.into_iter().collect();
                        assert_eq!(unique.len(), usize::from(edges));
                        txn.cancel().await?;
                    }
                    println!("{label}: edges={edges}, {n} lookups in {:?}", started.elapsed());
                }
            }
            Ok::<_, Box<dyn std::error::Error>>(())
        })
}
