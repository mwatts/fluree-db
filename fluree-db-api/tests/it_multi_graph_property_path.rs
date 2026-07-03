//! Multi-graph property-path traversal over a cross-ledger dataset (INDEXED).
//!
//! Mirrors the server's real conditions: data is pushed into the **binary
//! index** (not just novelty), then a cross-ledger dataset query is run through
//! the connection path. This is what triggers the multi-graph GRAPH + property
//! path failures that the novelty-only path does not.
//!
//! See the fluree/db issue "Property paths can't be combined with cross-ledger
//! queries". `q1`/`q2` characterize current (correct/guarded) behavior; `q3a`/
//! `q3b` assert the *desired* behavior of the `use GRAPH` escape hatch.

#![cfg(feature = "native")]

mod support;

use fluree_db_api::{FlureeBuilder, IndexConfig, LedgerManagerConfig};
use fluree_db_transact::{CommitOpts, TxnOpts};
use serde_json::{json, Value as JsonValue};
use support::{
    genesis_ledger_for_fluree, start_background_indexer_local, trigger_index_and_wait_outcome,
};

type MemoryFluree = fluree_db_api::Fluree;

async fn insert_indexed(
    fluree: &MemoryFluree,
    handle: &fluree_db_indexer::IndexerHandle,
    ledger_id: &str,
    doc: &JsonValue,
) {
    let index_cfg = IndexConfig {
        reindex_min_bytes: 0,
        reindex_max_bytes: 10_000_000,
    };
    let ledger = genesis_ledger_for_fluree(fluree, ledger_id);
    let result = fluree
        .insert_with_opts(
            ledger,
            doc,
            TxnOpts::default(),
            CommitOpts::default(),
            &index_cfg,
        )
        .await
        .expect("insert");
    let _ = trigger_index_and_wait_outcome(handle, ledger_id, result.ledger.t()).await;
}

async fn seed(fluree: &MemoryFluree, handle: &fluree_db_indexer::IndexerHandle) {
    insert_indexed(
        fluree,
        handle,
        "taxonomy:main",
        &json!({
            "@context": {"ex": "https://example.org/",
                         "rdfs": "http://www.w3.org/2000/01/rdf-schema#"},
            "@graph": [
                {"@id": "ex:top",    "rdfs:label": "Top"},
                {"@id": "ex:mid",    "ex:broader": {"@id": "ex:top"}, "rdfs:label": "Mid"},
                {"@id": "ex:narrow", "ex:broader": {"@id": "ex:mid"}, "rdfs:label": "Narrow"}
            ]
        }),
    )
    .await;
    insert_indexed(
        fluree,
        handle,
        "catalog:main",
        &json!({
            "@context": {"ex": "https://example.org/"},
            "@graph": [ {"@id": "ex:thing", "ex:category": {"@id": "ex:narrow"}} ]
        }),
    )
    .await;
}

fn fluree_with_indexer() -> (
    MemoryFluree,
    tokio::task::LocalSet,
    fluree_db_indexer::IndexerHandle,
) {
    let fluree = FlureeBuilder::memory()
        .with_ledger_cache_config(LedgerManagerConfig::default())
        .build_memory();
    let (local, handle) = start_background_indexer_local(
        fluree.backend().clone(),
        fluree
            .nameservice_mode()
            .as_arc_indexing_nameservice()
            .expect("test fluree has writable nameservice"),
        fluree_db_indexer::IndexerConfig::small(),
    );
    (fluree, local, handle)
}

/// Q1 (control) — property path over a SINGLE-graph `FROM`, indexed.
#[tokio::test]
async fn q1_single_graph_property_path_works() {
    let (fluree, local, handle) = fluree_with_indexer();
    local
        .run_until(async move {
            seed(&fluree, &handle).await;
            let sparql = r"
PREFIX ex: <https://example.org/>
SELECT ?anc FROM <taxonomy:main>
WHERE { ex:narrow ex:broader* ?anc }";
            let result = fluree
                .query_connection_sparql(sparql)
                .await
                .expect("single-graph property path should execute");
            let tax = fluree.ledger("taxonomy:main").await.expect("load");
            let s = result
                .to_jsonld(&tax.snapshot)
                .expect("to_jsonld")
                .to_string();
            assert!(
                s.contains("ex:narrow") && s.contains("ex:mid") && s.contains("ex:top"),
                "{s}"
            );
        })
        .await;
}

/// Q2 (characterization) — property path over MULTI-graph `FROM` is guarded.
#[tokio::test]
async fn q2_multi_graph_property_path_is_guarded() {
    let (fluree, local, handle) = fluree_with_indexer();
    local
        .run_until(async move {
            seed(&fluree, &handle).await;
            let sparql = r"
PREFIX ex: <https://example.org/>
SELECT DISTINCT ?thing FROM <catalog:main> FROM <taxonomy:main>
WHERE { ?thing ex:category ?c . ?c ex:broader* ex:top }";
            let err = fluree
                .query_connection_sparql(sparql)
                .await
                .expect_err("multi-graph property path should be rejected");
            assert!(
                err.to_string()
                    .contains("Property paths over multi-graph datasets are not supported"),
                "unexpected error: {err}"
            );
        })
        .await;
}

/// Q3a (BUG) — GRAPH-scoped property path over an INDEXED multi-ledger dataset
/// should join back to the default-graph instance. Expected: `ex:thing`.
#[tokio::test]
async fn q3a_graph_scoped_path_over_multiledger_should_join() {
    let (fluree, local, handle) = fluree_with_indexer();
    local
        .run_until(async move {
            seed(&fluree, &handle).await;
            let sparql = r"
PREFIX ex: <https://example.org/>
SELECT DISTINCT ?thing FROM <catalog:main> FROM NAMED <taxonomy:main>
WHERE { ?thing ex:category ?c . GRAPH <taxonomy:main> { ?c ex:broader* ex:top } }";
            let result = fluree.query_connection_sparql(sparql).await;
            assert!(
                result.is_ok(),
                "GRAPH-scoped property path over a multi-ledger dataset should execute, got: {:?}",
                result.err()
            );
            let cat = fluree.ledger("catalog:main").await.expect("load");
            let s = result
                .unwrap()
                .to_jsonld(&cat.snapshot)
                .expect("to_jsonld")
                .to_string();
            assert!(s.contains("ex:thing"), "expected ex:thing: {s}");
        })
        .await;
}

/// Q3c (BUG?) — same cross-graph join, but with **namespace-code divergence**:
/// the join-key namespace (`https://example.org/`) is registered first in
/// `taxonomy` but only later (via a ref) in `catalog2`, so it gets a different
/// code in each ledger. If the GRAPH-boundary join compares raw SIDs without
/// re-encoding (the #1295 family), `?c` won't match and this returns empty.
/// Expected: `cat:thing`.
#[tokio::test]
async fn q3c_cross_graph_join_divergent_ns_should_return_rows() {
    let (fluree, local, handle) = fluree_with_indexer();
    local
        .run_until(async move {
            // taxonomy: example.org registered FIRST (low code).
            insert_indexed(
                &fluree, &handle, "taxonomy:main",
                &json!({"@context": {"ex": "https://example.org/"},
                        "@graph": [
                            {"@id": "ex:narrow", "ex:broader": {"@id": "ex:mid"}},
                            {"@id": "ex:mid",    "ex:broader": {"@id": "ex:top"}}
                        ]}),
            ).await;
            // catalog2: catalog.example registered first, example.org only via the
            // ref to ex:narrow → example.org gets a *different* code here.
            insert_indexed(
                &fluree, &handle, "catalog2:main",
                &json!({"@context": {"cat": "https://catalog.example/", "ex": "https://example.org/"},
                        "@graph": [ {"@id": "cat:thing", "cat:category": {"@id": "ex:narrow"}} ]}),
            ).await;

            let sparql = r"
PREFIX ex: <https://example.org/>
PREFIX cat: <https://catalog.example/>
SELECT DISTINCT ?thing FROM NAMED <catalog2:main> FROM NAMED <taxonomy:main>
WHERE { GRAPH <catalog2:main> { ?thing cat:category ?c }
        GRAPH <taxonomy:main> { ?c ex:broader ex:mid } }";
            let result = fluree.query_connection_sparql(sparql).await;
            assert!(result.is_ok(), "should execute, got: {:?}", result.err());
            let cat = fluree.ledger("catalog2:main").await.expect("load");
            let s = result.unwrap().to_jsonld(&cat.snapshot).expect("to_jsonld").to_string();
            assert!(s.contains("cat:thing"), "divergent-ns cross-graph join returned no rows: {s}");
        })
        .await;
}

/// Q3d (MITIGATION) — same divergent setup as q3c, but `catalog3` is seeded
/// with a deterministic *vocabulary warm-up*: it touches the shared
/// `https://example.org/` namespace FIRST (before its own `cat:` namespace),
/// so example.org gets the SAME code as in `taxonomy`. If aligning codes
/// sidesteps the cross-graph re-encoding gap, this should return `cat:thing`.
#[tokio::test]
async fn q3d_namespace_warmup_aligns_codes_and_join_works() {
    let (fluree, local, handle) = fluree_with_indexer();
    local
        .run_until(async move {
            insert_indexed(
                &fluree, &handle, "taxonomy:main",
                &json!({"@context": {"ex": "https://example.org/"},
                        "@graph": [
                            {"@id": "ex:narrow", "ex:broader": {"@id": "ex:mid"}},
                            {"@id": "ex:mid",    "ex:broader": {"@id": "ex:top"}}
                        ]}),
            ).await;
            // WARM-UP: register example.org FIRST via a throwaway vocab node,
            // THEN the catalog-specific (cat:) data. example.org now aligns.
            insert_indexed(
                &fluree, &handle, "catalog3:main",
                &json!({"@context": {"ex": "https://example.org/", "cat": "https://catalog.example/"},
                        "@graph": [
                            {"@id": "ex:_vocab", "ex:_seed": "1"},
                            {"@id": "cat:thing", "cat:category": {"@id": "ex:narrow"}}
                        ]}),
            ).await;

            let sparql = r"
PREFIX ex: <https://example.org/>
PREFIX cat: <https://catalog.example/>
SELECT DISTINCT ?thing FROM NAMED <catalog3:main> FROM NAMED <taxonomy:main>
WHERE { GRAPH <catalog3:main> { ?thing cat:category ?c }
        GRAPH <taxonomy:main> { ?c ex:broader ex:mid } }";
            let result = fluree.query_connection_sparql(sparql).await;
            assert!(result.is_ok(), "should execute, got: {:?}", result.err());
            let cat = fluree.ledger("catalog3:main").await.expect("load");
            let s = result.unwrap().to_jsonld(&cat.snapshot).expect("to_jsonld").to_string();
            assert!(s.contains("cat:thing"), "warm-up did NOT align codes: {s}");
        })
        .await;
}

/// Q3b (BUG) — a variable bound inside a GRAPH block should join across the
/// boundary over an INDEXED multi-ledger dataset. Expected: `ex:thing`.
#[tokio::test]
async fn q3b_cross_graph_join_should_return_rows() {
    let (fluree, local, handle) = fluree_with_indexer();
    local
        .run_until(async move {
            seed(&fluree, &handle).await;
            let sparql = r"
PREFIX ex: <https://example.org/>
SELECT DISTINCT ?thing FROM NAMED <catalog:main> FROM NAMED <taxonomy:main>
WHERE { GRAPH <catalog:main> { ?thing ex:category ?c }
        GRAPH <taxonomy:main> { ?c ex:broader ex:mid } }";
            let result = fluree.query_connection_sparql(sparql).await;
            assert!(
                result.is_ok(),
                "cross-graph join should execute, got: {:?}",
                result.err()
            );
            let cat = fluree.ledger("catalog:main").await.expect("load");
            let s = result
                .unwrap()
                .to_jsonld(&cat.snapshot)
                .expect("to_jsonld")
                .to_string();
            assert!(
                s.contains("ex:thing"),
                "cross-graph join returned no rows: {s}"
            );
        })
        .await;
}

// =============================================================================
// Usage-pattern matrix (issue #1405, bugs 2+3). Each new case is INDEXED and
// uses DIVERGENT namespace codes (the ledger registers its own namespace first,
// the shared one only via a ref — so the shared namespace gets a different code
// per ledger), unless noted. These pin behaviors the q1–q3d repro does not.
// =============================================================================

/// A1 (P2 — join independent datasets) — join key bound as SUBJECT in graph 1,
/// used as OBJECT in graph 2, under namespace divergence. Exercises the
/// object-position substitution arm the subject-position repro (q3c) does not.
/// Expected: `org:acme`.
#[tokio::test]
async fn a1_object_position_cross_graph_join_divergent_ns() {
    let (fluree, local, handle) = fluree_with_indexer();
    local
        .run_until(async move {
            // staff: example.org registered FIRST → low code; ex:alice is a subject.
            insert_indexed(
                &fluree,
                &handle,
                "staff:main",
                &json!({"@context": {"ex": "https://example.org/"},
                        "@graph": [{"@id": "ex:alice", "@type": "ex:Engineer"}]}),
            )
            .await;
            // orgs: org.example registered first; example.org only via the employs
            // ref → a different code. ex:alice appears as the OBJECT of org:employs.
            insert_indexed(
                &fluree,
                &handle,
                "orgs:main",
                &json!({"@context": {"org": "https://org.example/", "ex": "https://example.org/"},
                        "@graph": [{"@id": "org:acme", "org:employs": {"@id": "ex:alice"}}]}),
            )
            .await;

            let sparql = r"
PREFIX ex: <https://example.org/>
PREFIX org: <https://org.example/>
SELECT DISTINCT ?org FROM NAMED <staff:main> FROM NAMED <orgs:main>
WHERE { GRAPH <staff:main> { ?p a ex:Engineer }
        GRAPH <orgs:main>  { ?org org:employs ?p } }";
            let result = fluree.query_connection_sparql(sparql).await;
            assert!(result.is_ok(), "should execute, got: {:?}", result.err());
            let orgs = fluree.ledger("orgs:main").await.expect("load");
            let s = result
                .unwrap()
                .to_jsonld(&orgs.snapshot)
                .expect("to_jsonld")
                .to_string();
            assert!(
                s.contains("org:acme"),
                "object-position divergent-ns join returned no rows: {s}"
            );
        })
        .await;
}

/// A2 (P4 — completeness) — an instance in TWO matching categories under
/// divergence: BOTH must come back (per-value re-encode, nothing dropped).
/// Expected: labels `Narrow` AND `Mid`.
#[tokio::test]
async fn a2_multi_value_cross_graph_join_divergent_ns() {
    let (fluree, local, handle) = fluree_with_indexer();
    local
        .run_until(async move {
            insert_indexed(
                &fluree,
                &handle,
                "taxonomy:main",
                &json!({"@context": {"ex": "https://example.org/",
                                     "rdfs": "http://www.w3.org/2000/01/rdf-schema#"},
                        "@graph": [
                            {"@id": "ex:mid", "rdfs:label": "Mid"},
                            {"@id": "ex:narrow", "rdfs:label": "Narrow"}
                        ]}),
            )
            .await;
            insert_indexed(
                &fluree,
                &handle,
                "catm:main",
                &json!({"@context": {"cat": "https://catalog.example/", "ex": "https://example.org/"},
                        "@graph": [{"@id": "cat:thing",
                                    "cat:category": [{"@id": "ex:narrow"}, {"@id": "ex:mid"}]}]}),
            )
            .await;

            let sparql = r"
PREFIX ex: <https://example.org/>
PREFIX cat: <https://catalog.example/>
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
SELECT DISTINCT ?label FROM NAMED <catm:main> FROM NAMED <taxonomy:main>
WHERE { GRAPH <catm:main> { cat:thing cat:category ?c }
        GRAPH <taxonomy:main> { ?c rdfs:label ?label } }";
            let result = fluree.query_connection_sparql(sparql).await;
            assert!(result.is_ok(), "should execute, got: {:?}", result.err());
            let cat = fluree.ledger("catm:main").await.expect("load");
            let s = result.unwrap().to_jsonld(&cat.snapshot).expect("to_jsonld").to_string();
            assert!(
                s.contains("Narrow") && s.contains("Mid"),
                "multi-value divergent-ns join dropped a value (want both Narrow+Mid): {s}"
            );
        })
        .await;
}

/// A3 (P4 — precision) — of two categories, only `ex:mid` has `ex:broader ex:top`
/// (`ex:narrow`'s broader is `ex:mid`). The divergent-ns join must return EXACTLY
/// `ex:mid` and must NOT falsely match `ex:narrow`. Guards against a re-encode so
/// loose it over-matches.
#[tokio::test]
async fn a3_cross_graph_join_precision_divergent_ns() {
    let (fluree, local, handle) = fluree_with_indexer();
    local
        .run_until(async move {
            insert_indexed(
                &fluree,
                &handle,
                "taxonomy:main",
                &json!({"@context": {"ex": "https://example.org/"},
                        "@graph": [
                            {"@id": "ex:mid", "ex:broader": {"@id": "ex:top"}},
                            {"@id": "ex:narrow", "ex:broader": {"@id": "ex:mid"}}
                        ]}),
            )
            .await;
            insert_indexed(
                &fluree,
                &handle,
                "catp:main",
                &json!({"@context": {"cat": "https://catalog.example/", "ex": "https://example.org/"},
                        "@graph": [{"@id": "cat:thing",
                                    "cat:category": [{"@id": "ex:narrow"}, {"@id": "ex:mid"}]}]}),
            )
            .await;

            let sparql = r"
PREFIX ex: <https://example.org/>
PREFIX cat: <https://catalog.example/>
SELECT DISTINCT ?c FROM NAMED <catp:main> FROM NAMED <taxonomy:main>
WHERE { GRAPH <catp:main> { cat:thing cat:category ?c }
        GRAPH <taxonomy:main> { ?c ex:broader ex:top } }";
            let result = fluree.query_connection_sparql(sparql).await;
            assert!(result.is_ok(), "should execute, got: {:?}", result.err());
            let tax = fluree.ledger("taxonomy:main").await.expect("load");
            let s = result.unwrap().to_jsonld(&tax.snapshot).expect("to_jsonld").to_string();
            assert!(
                s.contains("ex:mid"),
                "precision join missed the true match ex:mid: {s}"
            );
            assert!(
                !s.contains("ex:narrow"),
                "precision join falsely matched ex:narrow (over-match): {s}"
            );
        })
        .await;
}

/// A4 (P1 — taxonomy + instances) — `p+` (strict "proper ancestors") scoped path
/// joined to a default-graph instance, under divergence. Exercises bug 2 (path in
/// GRAPH, indexed) AND bug 3 (divergent join key) together. Expected: `cat:thing`.
#[tokio::test]
async fn a4_strict_path_plus_join_divergent_ns() {
    let (fluree, local, handle) = fluree_with_indexer();
    local
        .run_until(async move {
            insert_indexed(
                &fluree,
                &handle,
                "taxonomy:main",
                &json!({"@context": {"ex": "https://example.org/"},
                        "@graph": [
                            {"@id": "ex:narrow", "ex:broader": {"@id": "ex:mid"}},
                            {"@id": "ex:mid", "ex:broader": {"@id": "ex:top"}}
                        ]}),
            )
            .await;
            insert_indexed(
                &fluree,
                &handle,
                "cata:main",
                &json!({"@context": {"cat": "https://catalog.example/", "ex": "https://example.org/"},
                        "@graph": [{"@id": "cat:thing", "cat:category": {"@id": "ex:narrow"}}]}),
            )
            .await;

            let sparql = r"
PREFIX ex: <https://example.org/>
PREFIX cat: <https://catalog.example/>
SELECT DISTINCT ?thing FROM <cata:main> FROM NAMED <taxonomy:main>
WHERE { ?thing cat:category ?c . GRAPH <taxonomy:main> { ?c ex:broader+ ex:top } }";
            let result = fluree.query_connection_sparql(sparql).await;
            assert!(result.is_ok(), "should execute, got: {:?}", result.err());
            let cat = fluree.ledger("cata:main").await.expect("load");
            let s = result.unwrap().to_jsonld(&cat.snapshot).expect("to_jsonld").to_string();
            assert!(
                s.contains("cat:thing"),
                "strict-path + divergent-ns join returned no rows: {s}"
            );
        })
        .await;
}

/// A5 (P3 — chained hop) — a join key crossing TWO ledger boundaries
/// (app → catalog → upper), all with divergent codes on the shared `sh:`
/// namespace. Pins that re-encoding COMPOSES across more than one boundary.
/// Expected: `Upper A`.
#[tokio::test]
async fn a5_three_ledger_chain_divergent_ns() {
    let (fluree, local, handle) = fluree_with_indexer();
    local
        .run_until(async move {
            // Each ledger registers its own local namespace FIRST so the shared
            // `sh:` namespace gets a different code in each.
            insert_indexed(
                &fluree,
                &handle,
                "appl:main",
                &json!({"@context": {"l1": "https://l1.example/", "sh": "https://shared.example/"},
                "@graph": [
                    {"@id": "l1:_seed", "l1:x": "1"},
                    {"@id": "sh:item1", "sh:inCategory": {"@id": "sh:catX"}}
                ]}),
            )
            .await;
            insert_indexed(
                &fluree,
                &handle,
                "catl:main",
                &json!({"@context": {"l2": "https://l2.example/", "sh": "https://shared.example/"},
                "@graph": [
                    {"@id": "l2:_seed", "l2:x": "1"},
                    {"@id": "sh:catX", "sh:mapsTo": {"@id": "sh:upA"}}
                ]}),
            )
            .await;
            insert_indexed(
                &fluree,
                &handle,
                "upl:main",
                &json!({"@context": {"l3": "https://l3.example/", "sh": "https://shared.example/"},
                "@graph": [
                    {"@id": "l3:_seed", "l3:x": "1"},
                    {"@id": "sh:upA", "sh:label": "Upper A"}
                ]}),
            )
            .await;

            let sparql = r"
PREFIX sh: <https://shared.example/>
SELECT DISTINCT ?l FROM NAMED <appl:main> FROM NAMED <catl:main> FROM NAMED <upl:main>
WHERE { GRAPH <appl:main> { sh:item1 sh:inCategory ?c }
        GRAPH <catl:main> { ?c sh:mapsTo ?u }
        GRAPH <upl:main>  { ?u sh:label ?l } }";
            let result = fluree.query_connection_sparql(sparql).await;
            assert!(result.is_ok(), "should execute, got: {:?}", result.err());
            let up = fluree.ledger("upl:main").await.expect("load");
            let s = result
                .unwrap()
                .to_jsonld(&up.snapshot)
                .expect("to_jsonld")
                .to_string();
            assert!(
                s.contains("Upper A"),
                "three-ledger chained divergent-ns join returned no rows: {s}"
            );
        })
        .await;
}

/// A6 (regression) — a SINGLE-ledger GRAPH-scoped path is unaffected by the
/// multi-ledger binary-store gating (Fix 1 fires only when stamping is needed).
/// Green before and after. Expected: `ex:narrow`, `ex:mid`, `ex:top`.
#[tokio::test]
async fn a6_single_ledger_graph_path_unaffected() {
    let (fluree, local, handle) = fluree_with_indexer();
    local
        .run_until(async move {
            insert_indexed(
                &fluree,
                &handle,
                "taxonomy:main",
                &json!({"@context": {"ex": "https://example.org/"},
                "@graph": [
                    {"@id": "ex:narrow", "ex:broader": {"@id": "ex:mid"}},
                    {"@id": "ex:mid", "ex:broader": {"@id": "ex:top"}}
                ]}),
            )
            .await;

            let sparql = r"
PREFIX ex: <https://example.org/>
SELECT DISTINCT ?anc FROM NAMED <taxonomy:main>
WHERE { GRAPH <taxonomy:main> { ex:narrow ex:broader* ?anc } }";
            let result = fluree.query_connection_sparql(sparql).await;
            assert!(
                result.is_ok(),
                "single-ledger GRAPH path should execute, got: {:?}",
                result.err()
            );
            let tax = fluree.ledger("taxonomy:main").await.expect("load");
            let s = result
                .unwrap()
                .to_jsonld(&tax.snapshot)
                .expect("to_jsonld")
                .to_string();
            assert!(
                s.contains("ex:narrow") && s.contains("ex:mid") && s.contains("ex:top"),
                "single-ledger GRAPH path did not return the full chain: {s}"
            );
        })
        .await;
}

/// A7 (characterization) — `FILTER EXISTS` across a GRAPH boundary (semi-join).
/// This path is `SeedOperator`-based, distinct from the nested-loop join, so the
/// bug-2/bug-3 fixes may NOT cover it. Asserts the DESIRED behavior (the instance
/// whose category has `ex:broader ex:mid` is kept); if it fails after the fixes,
/// mark `#[ignore]` and file a semi-join follow-up. Divergent codes.
#[tokio::test]
async fn a7_filter_exists_cross_graph_divergent_ns() {
    let (fluree, local, handle) = fluree_with_indexer();
    local
        .run_until(async move {
            insert_indexed(
                &fluree,
                &handle,
                "taxonomy:main",
                &json!({"@context": {"ex": "https://example.org/"},
                        "@graph": [{"@id": "ex:narrow", "ex:broader": {"@id": "ex:mid"}}]}),
            )
            .await;
            insert_indexed(
                &fluree,
                &handle,
                "catx:main",
                &json!({"@context": {"cat": "https://catalog.example/", "ex": "https://example.org/"},
                        "@graph": [{"@id": "cat:thing", "cat:category": {"@id": "ex:narrow"}}]}),
            )
            .await;

            let sparql = r"
PREFIX ex: <https://example.org/>
PREFIX cat: <https://catalog.example/>
SELECT DISTINCT ?thing FROM NAMED <catx:main> FROM NAMED <taxonomy:main>
WHERE { GRAPH <catx:main> { ?thing cat:category ?c }
        FILTER EXISTS { GRAPH <taxonomy:main> { ?c ex:broader ex:mid } } }";
            let result = fluree.query_connection_sparql(sparql).await;
            assert!(result.is_ok(), "should execute, got: {:?}", result.err());
            let cat = fluree.ledger("catx:main").await.expect("load");
            let s = result.unwrap().to_jsonld(&cat.snapshot).expect("to_jsonld").to_string();
            assert!(
                s.contains("cat:thing"),
                "FILTER EXISTS across a GRAPH boundary (divergent ns) dropped the row: {s}"
            );
        })
        .await;
}

/// A8 (P1 — taxonomy crawl) — a BOTH-endpoints-unbound closure (`?s ex:broader+
/// ?o`) inside a GRAPH block, where the query's primary ledger differs from the
/// path's graph so the path predicate `ex:broader` has a divergent code. Pins
/// that the closure/adjacency read path (not just the bounded read_step) also
/// re-encodes the traversal predicate. Expected: ancestor pairs incl. ex:top.
#[tokio::test]
async fn a8_unbounded_closure_in_graph_divergent_pred() {
    let (fluree, local, handle) = fluree_with_indexer();
    local
        .run_until(async move {
            // primary (first FROM NAMED): registers cat: first, ex: only via a
            // ref → ex: gets a divergent code vs taxonomy.
            insert_indexed(
                &fluree,
                &handle,
                "prim:main",
                &json!({"@context": {"cat": "https://catalog.example/", "ex": "https://example.org/"},
                        "@graph": [{"@id": "cat:x", "cat:ref": {"@id": "ex:narrow"}}]}),
            )
            .await;
            insert_indexed(
                &fluree,
                &handle,
                "taxonomy:main",
                &json!({"@context": {"ex": "https://example.org/"},
                        "@graph": [
                            {"@id": "ex:narrow", "ex:broader": {"@id": "ex:mid"}},
                            {"@id": "ex:mid", "ex:broader": {"@id": "ex:top"}}
                        ]}),
            )
            .await;

            let sparql = r"
PREFIX ex: <https://example.org/>
SELECT DISTINCT ?s ?o FROM NAMED <prim:main> FROM NAMED <taxonomy:main>
WHERE { GRAPH <taxonomy:main> { ?s ex:broader+ ?o } }";
            let result = fluree.query_connection_sparql(sparql).await;
            assert!(result.is_ok(), "should execute, got: {:?}", result.err());
            let tax = fluree.ledger("taxonomy:main").await.expect("load");
            let s = result.unwrap().to_jsonld(&tax.snapshot).expect("to_jsonld").to_string();
            // narrow→mid→top: closure must include the deep pair reaching ex:top.
            assert!(
                s.contains("ex:top") && s.contains("ex:narrow"),
                "unbounded closure with a divergent-code predicate found no edges: {s}"
            );
        })
        .await;
}
