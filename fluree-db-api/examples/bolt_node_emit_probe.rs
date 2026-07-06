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
                "ex:knows": (1..=10).map(|k| json!({"@id": format!("ex:u{}", (i + k * 7) % users)})).collect::<Vec<_>>()
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

    // Correctness spot-check on the hydrated output (exercises the batched
    // crawl lane on this indexed, overlay-free ledger).
    use fluree_db_api::format::cypher_typed::CypherCell;
    let node = rows
        .iter()
        .flat_map(|r| r.iter())
        .find_map(|c| match c {
            CypherCell::Node(n) if n.iri == "http://example.org/u0" => Some(n),
            _ => None,
        })
        .expect("u0 in result");
    assert_eq!(node.labels, vec!["User"], "labels");
    let prop = |k: &str| {
        node.properties
            .iter()
            .find(|(key, _)| key == k)
            .map(|(_, v)| v)
    };
    assert_eq!(
        prop("name"),
        Some(&CypherCell::Value(serde_json::json!("user0")))
    );
    assert_eq!(prop("age"), Some(&CypherCell::Value(serde_json::json!(18))));
    assert_eq!(
        prop("region"),
        Some(&CypherCell::Value(serde_json::json!("region0")))
    );
    assert!(prop("knows").is_none(), "edges must not inline");
    assert_eq!(
        node.properties.len(),
        8,
        "8 scalar props, no refs: {:?}",
        node.properties
    );
    eprintln!("hydrated content verified (labels, scalars, no adjacency)");

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

    // Bound-edge-var pattern (task A): value-only edge vars add an OPTIONAL
    // annotation probe per hop unless the gate proves no reified edges exist.
    let pattern_q = "MATCH (a:User {id: 5})-[e1]->(b:User)-[e2]->(c:User) RETURN count(c) AS n";
    let mut total = 0f64;
    for _ in 0..iters {
        let t0 = Instant::now();
        std::hint::black_box(fluree.query_cypher(&db, pattern_q).await.expect("pattern"));
        total += t0.elapsed().as_secs_f64();
    }
    eprintln!(
        "bound-edge 2-hop pattern: {:.3} ms/iter",
        total * 1000.0 / iters as f64
    );

    // shortestPath (bidirectional BFS): per-node range reads today.
    let sp_q =
        "MATCH p = shortestPath((a:User {id: 5})-[*..15]->(b:User {id: 137})) RETURN length(p)";
    let mut total = 0f64;
    for _ in 0..iters {
        let t0 = Instant::now();
        std::hint::black_box(fluree.query_cypher(&db, sp_q).await.expect("sp"));
        total += t0.elapsed().as_secs_f64();
    }
    eprintln!(
        "shortestPath wildcard: {:.3} ms/iter",
        total * 1000.0 / iters as f64
    );

    // --- Live-novelty phase (the benchmark condition: writes ran, no
    // reindex). The per-subject gate must keep untouched subjects on the
    // batched lane, and touched subjects must render merged truth.
    let ledger = fluree.ledger("probe:main").await.expect("ledger");
    let res = fluree
        .insert(
            ledger,
            &json!({
                "@context": {"ex": "http://example.org/"},
                "@graph": [
                    {"@id": "ex:newcomer", "@type": "ex:User", "ex:id": 9999, "ex:name": "newcomer",
                     "ex:age": 99, "ex:gender": 1, "ex:region": "region0", "ex:cmpl": 1,
                     "ex:eyes": "color0", "ex:hair": "hair0"},
                    {"@id": "ex:u0", "ex:age": 77}
                ]
            }),
        )
        .await
        .expect("novelty insert");
    let db = fluree.db("probe:main").await.expect("db post-novelty");
    assert!(res.ledger.t() > 1, "novelty commit landed");

    let mut total = 0f64;
    for _ in 0..iters {
        let result = fluree.query_cypher(&db, query).await.expect("query");
        let t0 = Instant::now();
        std::hint::black_box(result.to_cypher_typed_table(&db).await.expect("typed"));
        total += t0.elapsed().as_secs_f64();
    }
    eprintln!(
        "typed  format under live novelty: {:.3} ms/iter",
        total * 1000.0 / iters as f64
    );

    // Correctness: u0 was touched by novelty (age 18 -> now ALSO 77 as a
    // multi-value assert) and must come off the merge-correct fallback;
    // u1 is untouched and must still be exact off the batched lane.
    let result = fluree.query_cypher(&db, query).await.expect("query");
    let (_, rows) = result.to_cypher_typed_table(&db).await.expect("typed");
    let find = |iri: &str| {
        rows.iter().flat_map(|r| r.iter()).find_map(|c| match c {
            CypherCell::Node(n) if n.iri == iri => Some(n.clone()),
            _ => None,
        })
    };
    let u0 = find("http://example.org/u0").expect("u0");
    let age = u0
        .properties
        .iter()
        .find(|(k, _)| k == "age")
        .map(|(_, v)| v.clone());
    match age {
        Some(CypherCell::List(vals)) => assert!(
            vals.contains(&CypherCell::Value(serde_json::json!(77))),
            "dirty subject must reflect novelty: {vals:?}"
        ),
        Some(CypherCell::Value(v)) => assert_eq!(v, serde_json::json!(77), "novelty age"),
        other => panic!("u0 age missing: {other:?}"),
    }
    let u1 = find("http://example.org/u1").expect("u1");
    assert_eq!(
        u1.properties
            .iter()
            .find(|(k, _)| k == "age")
            .map(|(_, v)| v.clone()),
        Some(CypherCell::Value(serde_json::json!(19))),
        "untouched subject exact off the batched lane"
    );
    let newcomer = find("http://example.org/newcomer").expect("novelty-only subject in result");
    assert_eq!(newcomer.labels, vec!["User"]);
    eprintln!(
        "live-novelty correctness verified (dirty=fallback, clean=batched, novelty-only present)"
    );
}
