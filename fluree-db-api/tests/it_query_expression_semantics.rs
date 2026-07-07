//! Expression-semantics regression tests (W3C burn-down PR-X1).
//!
//! Covers the cheap high-yield expression defects: constant FILTER placement
//! (#1439), DATATYPE()/LANG() expression arguments and SPARQL-scoped
//! non-literal type errors (#1440, decision D-12), xsd:dateTime/date/time
//! casts, BNODE per-solution identity, the regex `q` flag, and bare-integer
//! lowering (#1319).
//!
//! Per `docs/contributing/sparql-compliance.md` § Query Surface Parity these
//! fixes are IR/engine-level, so each one carries a JSON-LD-surface
//! regression alongside the SPARQL one — the W3C submodule only guards the
//! SPARQL surface. The D2b split is deliberate and asserted on BOTH surfaces:
//! SPARQL `DATATYPE`/`LANG` of a non-literal is a type error (row excluded /
//! variable unbound), while the JSON-LD surface keeps Fluree's documented
//! `@id`-datatype extension.

use crate::support;
use crate::support::{genesis_ledger, MemoryFluree, MemoryLedger};
use fluree_db_api::FlureeBuilder;
use serde_json::{json, Value as JsonValue};

fn ctx() -> JsonValue {
    json!({
        "ex": "http://example.org/ns/",
        "xsd": "http://www.w3.org/2001/XMLSchema#"
    })
}

async fn seed_people(fluree: &MemoryFluree, ledger_id: &str) -> MemoryLedger {
    let ledger0 = genesis_ledger(fluree, ledger_id);
    let tx = json!({
        "@context": ctx(),
        "@graph": [
            { "@id": "ex:alice", "ex:name": "Alice", "ex:age": 30 },
            { "@id": "ex:bob", "ex:name": "Bob", "ex:age": 25 }
        ]
    });
    fluree.insert(ledger0, &tx).await.expect("insert").ledger
}

async fn sparql_rows(fluree: &MemoryFluree, ledger: &MemoryLedger, q: &str) -> JsonValue {
    support::query_sparql(fluree, ledger, q)
        .await
        .expect("query")
        .to_jsonld_async(ledger.as_graph_db_ref(0))
        .await
        .expect("to_jsonld_async")
}

async fn jsonld_rows(fluree: &MemoryFluree, ledger: &MemoryLedger, q: &JsonValue) -> JsonValue {
    support::query_jsonld(fluree, ledger, q)
        .await
        .expect("query")
        .to_jsonld_async(ledger.as_graph_db_ref(0))
        .await
        .expect("to_jsonld_async")
}

// =============================================================================
// D1 — a variable-free FILTER must apply to the group's solutions (#1439)
// =============================================================================

#[tokio::test]
async fn sparql_constant_filter_keeps_all_rows() {
    let fluree = FlureeBuilder::memory().build_memory();
    let ledger = seed_people(&fluree, "exprsem/d1:sparql").await;

    // Constant-true filters must be no-ops for the whole group.
    for q in [
        "ASK { FILTER(1 = 1) }",
        "ASK { FILTER(true) }",
        "ASK { ?s ?p ?o . FILTER(1 = 1) }",
        "ASK { FILTER(2 IN (1, 2, 3)) }",
        "ASK { FILTER(2 NOT IN ()) }",
    ] {
        assert_eq!(
            sparql_rows(&fluree, &ledger, q).await,
            JsonValue::Bool(true),
            "{q}"
        );
    }

    // Constant-false filters must eliminate the whole group.
    for q in ["ASK { FILTER(1 = 2) }", "ASK { ?s ?p ?o . FILTER(false) }"] {
        assert_eq!(
            sparql_rows(&fluree, &ledger, q).await,
            JsonValue::Bool(false),
            "{q}"
        );
    }

    // The filter's position must not gate the row source it precedes.
    let rows = sparql_rows(
        &fluree,
        &ledger,
        "PREFIX ex: <http://example.org/ns/> \
         SELECT ?n WHERE { ?s ex:name ?n . FILTER(1 = 1) } ORDER BY ?n",
    )
    .await;
    assert_eq!(rows, json!([["Alice"], ["Bob"]]));

    let rows = sparql_rows(
        &fluree,
        &ledger,
        "SELECT ?v WHERE { VALUES ?v { 1 2 } FILTER(true) } ORDER BY ?v",
    )
    .await;
    assert_eq!(rows, json!([[1], [2]]));
}

#[tokio::test]
async fn jsonld_constant_filter_keeps_all_rows() {
    let fluree = FlureeBuilder::memory().build_memory();
    let ledger = seed_people(&fluree, "exprsem/d1:jsonld").await;

    let q = json!({
        "@context": ctx(),
        "select": ["?n"],
        "where": [
            { "@id": "?s", "ex:name": "?n" },
            ["filter", "(= 1 1)"]
        ],
        "orderBy": "?n"
    });
    assert_eq!(
        jsonld_rows(&fluree, &ledger, &q).await,
        json!([["Alice"], ["Bob"]])
    );

    let q_false = json!({
        "@context": ctx(),
        "select": ["?n"],
        "where": [
            { "@id": "?s", "ex:name": "?n" },
            ["filter", "(= 1 2)"]
        ]
    });
    assert_eq!(jsonld_rows(&fluree, &ledger, &q_false).await, json!([]));
}
