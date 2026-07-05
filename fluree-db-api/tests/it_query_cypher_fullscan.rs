// Cypher whole-graph-scan opt-in tests. These live in their own test binary
// because `FLUREE_CYPHER_ALLOW_FULL_SCAN` is read once per process — setting
// it here must not leak into the main Cypher tests (which assert the
// default rejection of bare `MATCH (n)`).
#![allow(clippy::needless_raw_string_hashes)]

mod support;

use fluree_db_api::FlureeBuilder;
use serde_json::json;
use support::{genesis_ledger, graphdb_from_ledger};

#[tokio::test]
async fn cypher_bare_match_full_scan_opt_in() {
    std::env::set_var("FLUREE_CYPHER_ALLOW_FULL_SCAN", "1");

    let fluree = FlureeBuilder::memory().build_memory();
    let ledger0 = genesis_ledger(&fluree, "it/cypher:full-scan");
    let committed = fluree
        .insert(
            ledger0,
            &json!({
                "@context": {"ex": "http://example.org/"},
                "@graph": [
                    {"@id": "ex:alice", "@type": "ex:Person", "ex:age": 25},
                    {"@id": "ex:bob",   "@type": "ex:Person"},
                    {"@id": "ex:acme",  "@type": "ex:Company", "ex:age": 99},
                ]
            }),
        )
        .await
        .expect("seed");
    let db = graphdb_from_ledger(&committed.ledger);

    // count(n) counts distinct nodes (subjects), not triples; count(n.age)
    // counts nodes carrying the property (benchgraph `aggregation__count`).
    let cj = fluree
        .query_cypher(&db, "MATCH (n) RETURN count(n) AS c, count(n.age) AS ca")
        .await
        .expect("bare MATCH scan")
        .to_cypher_json_async(db.as_graph_db_ref())
        .await
        .expect("cypher json");
    let row = &cj["results"][0]["data"][0]["row"];
    // 3 user nodes + 2 class nodes (ex:Person / ex:Company are subjects of
    // nothing — they appear only as objects, so they are not counted).
    assert_eq!(row[0], json!(3), "count(n): {cj}");
    assert_eq!(row[1], json!(2), "count(n.age): {cj}");

    // min/max/avg over a property through the scan
    // (benchgraph `aggregation__min_max_avg`).
    let cj = fluree
        .query_cypher(
            &db,
            "MATCH (n) RETURN min(n.age) AS mn, max(n.age) AS mx, avg(n.age) AS av",
        )
        .await
        .expect("min/max/avg scan")
        .to_cypher_json_async(db.as_graph_db_ref())
        .await
        .expect("cypher json");
    let row = &cj["results"][0]["data"][0]["row"];
    assert_eq!(row[0], json!(25), "min: {cj}");
    assert_eq!(row[1], json!(99), "max: {cj}");
    // avg yields xsd:decimal, which cypher-json renders as a string to
    // preserve precision.
    assert_eq!(row[2], json!("62"), "avg: {cj}");
}
