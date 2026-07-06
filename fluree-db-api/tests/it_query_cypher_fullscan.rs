// Cypher whole-graph-scan opt-in tests. These live in their own test binary
// because `FLUREE_CYPHER_ALLOW_FULL_SCAN` is read once per process — setting
// it here must not leak into the main Cypher tests (which assert the
// default rejection of bare `MATCH (n)`).
#![allow(clippy::needless_raw_string_hashes)]

mod support;

use fluree_db_api::FlureeBuilder;
use serde_json::json;
use support::{genesis_ledger, graphdb_from_ledger, rebuild_and_publish_index};

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

/// Run one Cypher query against both views and return the two cypher-json rows.
async fn row_on_both(
    fluree: &fluree_db_api::Fluree,
    novelty_db: &fluree_db_api::GraphDb,
    indexed_db: &fluree_db_api::GraphDb,
    cypher: &str,
) -> (serde_json::Value, serde_json::Value) {
    let mut rows = Vec::new();
    for db in [novelty_db, indexed_db] {
        let cj = fluree
            .query_cypher(db, cypher)
            .await
            .expect("query")
            .to_cypher_json_async(db.as_graph_db_ref())
            .await
            .expect("cypher json");
        rows.push(cj["results"][0]["data"][0]["row"].clone());
    }
    let indexed = rows.pop().unwrap();
    let novelty = rows.pop().unwrap();
    (novelty, indexed)
}

/// Whole-graph aggregates on an indexed ledger take the directory-fold fast
/// path (`fast_whole_graph_agg`); the same queries on the novelty-only view
/// run the general pipeline. Both must agree — including the row-multiplying
/// left-join semantics of a multi-valued property.
#[tokio::test]
async fn cypher_indexed_whole_graph_aggregates_match_pipeline() {
    std::env::set_var("FLUREE_CYPHER_ALLOW_FULL_SCAN", "1");

    let fluree = FlureeBuilder::memory().build_memory();
    let ledger_id = "it/cypher:full-scan-indexed";
    let ledger0 = genesis_ledger(&fluree, ledger_id);
    let committed = fluree
        .insert(
            ledger0,
            &json!({
                "@context": {"ex": "http://example.org/"},
                "@graph": [
                    // alice has TWO ages: the accessor left-join gives her two
                    // rows, so count(n) = 5 over 4 subjects while
                    // count(n.age) = 4 values.
                    {"@id": "ex:alice", "@type": "ex:Person", "ex:age": [25, 30]},
                    {"@id": "ex:bob",   "@type": "ex:Person"},
                    {"@id": "ex:carol", "@type": "ex:Person", "ex:age": 40},
                    {"@id": "ex:dave",  "@type": "ex:Person", "ex:age": 40},
                ]
            }),
        )
        .await
        .expect("seed");
    let novelty_db = graphdb_from_ledger(&committed.ledger);

    rebuild_and_publish_index(&fluree, ledger_id).await;
    let indexed_db = fluree.db(ledger_id).await.expect("indexed view");

    // count(n) / count(n.prop) / count(DISTINCT n) — exact integer folds.
    let (novelty, indexed) = row_on_both(
        &fluree,
        &novelty_db,
        &indexed_db,
        "MATCH (n) RETURN count(n) AS c, count(n.age) AS ca, count(DISTINCT n) AS cd",
    )
    .await;
    assert_eq!(novelty, json!([5, 4, 4]), "pipeline row");
    assert_eq!(indexed, json!([5, 4, 4]), "fold row");

    // min/max fold from POST boundary keys; avg from the predicate-scoped
    // scan. avg = (25+30+40+40)/4 = 33.75. The general pipeline renders the
    // decimal as a string while the fold (like the existing SPARQL AVG fast
    // path) yields a double — compare numerically.
    let (novelty, indexed) = row_on_both(
        &fluree,
        &novelty_db,
        &indexed_db,
        "MATCH (n) RETURN min(n.age) AS mn, max(n.age) AS mx, avg(n.age) AS av",
    )
    .await;
    for (label, row) in [("pipeline", &novelty), ("fold", &indexed)] {
        assert_eq!(row[0], json!(25), "{label} min: {row}");
        assert_eq!(row[1], json!(40), "{label} max: {row}");
        let avg = row[2]
            .as_f64()
            .or_else(|| row[2].as_str().and_then(|s| s.parse().ok()))
            .expect("numeric avg");
        assert!((avg - 33.75).abs() < 1e-9, "{label} avg: {row}");
    }

    // sum through the same fold.
    let (novelty, indexed) = row_on_both(
        &fluree,
        &novelty_db,
        &indexed_db,
        "MATCH (n) RETURN sum(n.age) AS s",
    )
    .await;
    assert_eq!(novelty, json!([135]), "pipeline sum");
    assert_eq!(indexed, json!([135]), "fold sum");

    // count(n) with no accessor = distinct subjects.
    let (novelty, indexed) = row_on_both(
        &fluree,
        &novelty_db,
        &indexed_db,
        "MATCH (n) RETURN count(n) AS c",
    )
    .await;
    assert_eq!(novelty, json!([4]), "pipeline count");
    assert_eq!(indexed, json!([4]), "fold count");
}

/// `f:reifies*` facts are hidden from the `?n ?p ?o` pipeline but present in
/// the SPOT directories, so their presence must make the fold decline to the
/// fallback — results stay identical to the novelty view either way.
#[tokio::test]
async fn cypher_indexed_whole_graph_count_declines_on_edge_annotations() {
    std::env::set_var("FLUREE_CYPHER_ALLOW_FULL_SCAN", "1");

    let fluree = FlureeBuilder::memory().build_memory();
    let ledger_id = "it/cypher:full-scan-annotated";
    let ledger0 = genesis_ledger(&fluree, ledger_id);
    let committed = fluree
        .insert(
            ledger0,
            &json!({
                "@context": {"ex": "http://example.org/"},
                "@graph": [
                    {
                        "@id": "ex:alice",
                        "@type": "ex:Person",
                        "ex:knows": {
                            "@id": "ex:bob",
                            "@annotation": {"ex:since": 2020}
                        }
                    },
                    {"@id": "ex:bob", "@type": "ex:Person"},
                ]
            }),
        )
        .await
        .expect("seed");
    let novelty_db = graphdb_from_ledger(&committed.ledger);

    rebuild_and_publish_index(&fluree, ledger_id).await;
    let indexed_db = fluree.db(ledger_id).await.expect("indexed view");

    let (novelty, indexed) = row_on_both(
        &fluree,
        &novelty_db,
        &indexed_db,
        "MATCH (n) RETURN count(n) AS c",
    )
    .await;
    assert_eq!(
        novelty, indexed,
        "annotated graph: fold must defer to the pipeline"
    );
}

/// Incrementally-built indexes currently persist delta-only per-graph
/// property stats (base entries lost; net-zero churn kept at count 0). The
/// folds must stay correct anyway: COUNT(DISTINCT ?p) always walks PSOT
/// directories, and the whole-graph fold's hidden-predicate gate checks the
/// predicate dictionary rather than the stats. This drives the corruption
/// trigger — full index, then writes (new predicate, net-zero churn, a
/// reified edge), then an incremental index — and pins fold == pipeline.
#[tokio::test]
async fn folds_stay_correct_on_incremental_index() {
    std::env::set_var("FLUREE_CYPHER_ALLOW_FULL_SCAN", "1");

    let fluree = FlureeBuilder::memory().build_memory();
    let ledger_id = "it/cypher:incremental-stats";
    let ledger0 = genesis_ledger(&fluree, ledger_id);
    let ctx = json!({"ex": "http://example.org/"});
    let ledger1 = fluree
        .insert(
            ledger0,
            &json!({
                "@context": ctx,
                "@graph": [
                    {"@id": "ex:alice", "@type": "ex:Person", "ex:age": 30, "ex:score": 1},
                    {"@id": "ex:bob",   "@type": "ex:Person", "ex:age": 40},
                ]
            }),
        )
        .await
        .expect("seed")
        .ledger;
    rebuild_and_publish_index(&fluree, ledger_id).await;

    // Post-index novelty: a brand-new predicate + class...
    let ledger2 = fluree
        .insert(
            ledger1,
            &json!({
                "@context": ctx,
                "@id": "ex:w1", "@type": "ex:Widget", "ex:brandnew": 7
            }),
        )
        .await
        .expect("new predicate")
        .ledger;
    // ...net-zero churn on ex:score (retract 1, assert 2 — count unchanged)...
    let ledger3 = fluree
        .update(
            ledger2,
            &json!({
                "@context": ctx,
                "delete": [{"@id": "ex:alice", "ex:score": 1}],
                "insert": [{"@id": "ex:alice", "ex:score": 2}]
            }),
        )
        .await
        .expect("churn")
        .ledger;
    // ...and a reified edge (f:reifies* enters the predicate dictionary).
    let _ledger4 = fluree
        .insert(
            ledger3,
            &json!({
                "@context": ctx,
                "@id": "ex:alice",
                "ex:knows": {"@id": "ex:bob", "@annotation": {"ex:since": 2020}}
            }),
        )
        .await
        .expect("annotated edge")
        .ledger;

    // Incremental index (base exists, small commit gap → incremental path).
    support::build_and_publish_index(&fluree, ledger_id).await;
    let db = fluree.db(ledger_id).await.expect("incremental view");

    // COUNT(DISTINCT ?p): the fold's PSOT directory walk vs the general
    // pipeline (GROUP BY blocks the distinct-count fold).
    let folded = fluree
        .query(
            &db,
            fluree_db_api::QueryInput::Sparql(
                "SELECT (COUNT(DISTINCT ?p) AS ?c) WHERE { ?s ?p ?o }",
            ),
        )
        .await
        .expect("distinct preds")
        .to_jsonld_async(db.as_graph_db_ref())
        .await
        .expect("jsonld");
    let truth = fluree
        .query(
            &db,
            fluree_db_api::QueryInput::Sparql("SELECT ?p WHERE { ?s ?p ?o } GROUP BY ?p"),
        )
        .await
        .expect("preds truth")
        .row_count();
    assert_eq!(
        folded[0][0].as_i64(),
        Some(truth as i64),
        "COUNT(DISTINCT ?p) fold vs pipeline: {folded} vs {truth}"
    );

    // Whole-graph count: reifies facts are in the dictionary, so the fold
    // must decline (dict-based gate) — and agree with the WITH-blocked
    // pipeline either way.
    let count_fold = fluree
        .query_cypher(&db, "MATCH (n) RETURN count(n) AS c")
        .await
        .expect("count")
        .to_cypher_json_async(db.as_graph_db_ref())
        .await
        .expect("cj");
    let count_truth = fluree
        .query_cypher(&db, "MATCH (n) WITH n RETURN count(n) AS c")
        .await
        .expect("count truth")
        .to_cypher_json_async(db.as_graph_db_ref())
        .await
        .expect("cj");
    assert_eq!(
        count_fold["results"][0]["data"][0]["row"], count_truth["results"][0]["data"][0]["row"],
        "whole-graph count on incremental+reified store"
    );

    // Class-anchored histogram still folds correctly on the incremental
    // index (class stats are maintained incrementally).
    let hist = fluree
        .query_cypher(&db, "MATCH (n:Person) RETURN n.age, COUNT(*)")
        .await
        .expect("hist")
        .to_cypher_json_async(db.as_graph_db_ref())
        .await
        .expect("cj");
    let rows = hist["results"][0]["data"].as_array().expect("rows");
    assert_eq!(rows.len(), 2, "{hist}");
}
