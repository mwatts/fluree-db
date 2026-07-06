//! Probe the Bolt node-emit path (BOLT-1b): time `to_cypher_typed_table`
//! (node hydration) against `to_cypher_table` (flat, no hydration) for a
//! `RETURN n` result over an **indexed** ledger — the binary-scan shape
//! whose bindings arrive as `EncodedSid`, matching the benchmark condition.
//!
//! ```bash
//! cargo run --release --example bolt_node_emit_probe -p fluree-db-api
//! ```

use std::time::Instant;

use fluree_db_api::{Fluree, FlureeBuilder, ReindexOptions};
use serde_json::json;

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    let users = env_usize("PROBE_USERS", 200);
    let iters = env_usize("PROBE_ITERS", 30);

    let dir = tempfile::tempdir().expect("tempdir");
    let fluree: Fluree = FlureeBuilder::file(dir.path().to_string_lossy().to_string())
        .build()
        .expect("fluree");
    let ledger = fluree.create_ledger("probe:main").await.expect("ledger");

    // Pokec-ish rich nodes: 8 scalar properties + a couple of refs.
    let graph: Vec<_> = (0..users)
        .map(|i| {
            json!({
                "@id": format!("ex:u{i}"),
                "@type": "ex:User",
                "ex:id": i,
                "ex:name": format!("user{i}"),
                "ex:age": 18 + (i % 60),
                "ex:gender": (i % 2),
                "ex:region": format!("region{}", i % 20),
                "ex:cmpl": (i % 100),
                "ex:eyes": format!("color{}", i % 5),
                "ex:hair": format!("hair{}", i % 7),
                "ex:knows": {"@id": format!("ex:u{}", (i + 1) % users)}
            })
        })
        .collect();
    fluree
        .insert(
            ledger,
            &json!({"@context": {"ex": "http://example.org/"}, "@graph": graph}),
        )
        .await
        .expect("seed");

    // Index the ledger so bindings come off the binary scan (EncodedSid).
    fluree
        .reindex("probe:main", ReindexOptions::default())
        .await
        .expect("reindex");

    let db = fluree.db("probe:main").await.expect("db");
    let query = "MATCH (n:User) WHERE n.age >= 18 RETURN n";

    let result = fluree.query_cypher(&db, query).await.expect("query");
    let encoded = result
        .batches
        .first()
        .and_then(|b| b.schema().first().map(|&v| b.get(0, v)))
        .flatten()
        .map(fluree_db_query::binding::Binding::is_encoded)
        .unwrap_or(false);
    let (_, rows) = result.to_cypher_typed_table(&db).await.expect("typed");
    eprintln!(
        "rows: {}, first binding encoded: {encoded} (encoded = benchmark condition)",
        rows.len()
    );

    // Flat table (what HTTP emits — no hydration), typed table (Bolt).
    for (name, typed) in [("flat  ", false), ("typed ", true)] {
        let mut total = 0f64;
        for _ in 0..iters {
            let result = fluree.query_cypher(&db, query).await.expect("query");
            let t0 = Instant::now();
            if typed {
                std::hint::black_box(result.to_cypher_typed_table(&db).await.expect("typed"));
            } else {
                std::hint::black_box(result.to_cypher_table(&db.snapshot).expect("flat"));
            }
            total += t0.elapsed().as_secs_f64();
        }
        eprintln!(
            "{name} format: {:.3} ms/iter",
            total * 1000.0 / iters as f64
        );
    }

    // End-to-end including query execution, for scale.
    let mut total = 0f64;
    for _ in 0..iters {
        let t0 = Instant::now();
        let result = fluree.query_cypher(&db, query).await.expect("query");
        std::hint::black_box(result.to_cypher_typed_table(&db).await.expect("typed"));
        total += t0.elapsed().as_secs_f64();
    }
    eprintln!(
        "query + typed: {:.3} ms/iter",
        total * 1000.0 / iters as f64
    );
}
