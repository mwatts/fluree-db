//! JSON-LD parity tests for PR-W1 (algebra scope & subquery correlation).
//!
//! The W3C submodule only guards the SPARQL surface, so these are the JSON-LD
//! (FQL) analogues of the three W3C tests PR-W1 greens. Families A and B live
//! in the shared IR/executor (`fluree-db-query`'s `subquery.rs`/`optional.rs`),
//! so the JSON-LD front-end must exhibit the same scope/correlation semantics.
//! See `docs/audit/burn-down/algebra-serialization.md`.
//!
//!   A — a FILTER in a nested scope must not see an enclosing-scope variable
//!       (W3C `filter-nested-2`, `dawg-optional-filter-005-not-simplified`).
//!   B — a sub-SELECT that binds a correlation variable only via OPTIONAL must
//!       reconcile it against the parent at the join, not pin it to the parent
//!       value (W3C `var-scope-join-1`, aka join-scope-1).

use crate::support::{genesis_ledger, graphdb_from_ledger};
use fluree_db_api::FlureeBuilder;
use serde_json::{json, Value as JsonValue};

fn ctx() -> JsonValue {
    json!({ "ex": "http://example.org/ns/" })
}

/// Family A — JSON-LD analogue of W3C `filter-nested-2`
/// (`{ :x :p ?v . { FILTER(?v = 1) } }` expects 0).
///
/// A FILTER inside a nested sub-SELECT references `?title`, which the sub-SELECT
/// does not select and therefore does not bind. Evaluated in its own scope
/// `?title` is unbound, so `(= ?title "T2")` is a type error → EBV false → the
/// sub-SELECT is empty → the join yields 0 rows. If the nested scope leaked the
/// enclosing `?title` (the defect PR-W1 fixes on the SPARQL surface), the FILTER
/// would match `ex:b2` and a row would survive.
#[tokio::test]
async fn nested_subquery_filter_does_not_see_enclosing_var() {
    let fluree = FlureeBuilder::memory().build_memory();
    let ledger0 = genesis_ledger(&fluree, "w1/parity:a1");
    let txn = json!({
        "@context": ctx(),
        "@graph": [
            {"@id": "ex:b1", "ex:title": "T1"},
            {"@id": "ex:b2", "ex:title": "T2"}
        ]
    });
    let db = graphdb_from_ledger(&fluree.insert(ledger0, &txn).await.expect("seed").ledger);

    let q = json!({
        "@context": ctx(),
        "select": ["?book", "?title"],
        "where": [
            {"@id": "?book", "ex:title": "?title"},
            ["query", {
                "@context": ctx(),
                "select": ["?book"],
                "where": [
                    {"@id": "?book", "ex:title": "?t2"},
                    ["filter", "(= ?title \"T2\")"]
                ]
            }]
        ]
    });

    let rows = fluree
        .query(&db, &q)
        .await
        .expect("query")
        .to_jsonld_async(db.as_graph_db_ref())
        .await
        .expect("format");

    assert_eq!(
        rows,
        json!([]),
        "the nested sub-SELECT's FILTER must see ?title as unbound (own scope), \
         so the whole query is empty; got {rows}"
    );
}

/// Family A — JSON-LD analogue of W3C `dawg-optional-filter-005-not-simplified`
/// (`{ ?book :title ?t . OPTIONAL { { ?book :price ?p . FILTER(?t = "TITLE 2") } } }`
/// expects every book title-only, no price).
///
/// The OPTIONAL wraps a nested sub-SELECT whose FILTER references the enclosing
/// `?title`. Because the sub-SELECT is an independent scope, `?title` is unbound
/// inside it → the FILTER errors → the OPTIONAL body is empty → no book is
/// assigned a price. (Were `?title` visible inside the nested scope, `ex:b2`
/// would wrongly acquire its price — the "not simplified" bug.)
#[tokio::test]
async fn optional_nested_filter_referencing_outer_var_does_not_bind() {
    let fluree = FlureeBuilder::memory().build_memory();
    let ledger0 = genesis_ledger(&fluree, "w1/parity:a2");
    let txn = json!({
        "@context": ctx(),
        "@graph": [
            {"@id": "ex:b1", "ex:title": "T1", "ex:price": 10},
            {"@id": "ex:b2", "ex:title": "T2", "ex:price": 20}
        ]
    });
    let db = graphdb_from_ledger(&fluree.insert(ledger0, &txn).await.expect("seed").ledger);

    let q = json!({
        "@context": ctx(),
        "select": ["?book", "?title", "?price"],
        "where": [
            {"@id": "?book", "ex:title": "?title"},
            ["optional",
                ["query", {
                    "@context": ctx(),
                    "select": ["?book", "?price"],
                    "where": [
                        {"@id": "?book", "ex:price": "?price"},
                        ["filter", "(= ?title \"T2\")"]
                    ]
                }]
            ]
        ],
        "orderBy": "?title"
    });

    let rows = fluree
        .query(&db, &q)
        .await
        .expect("query")
        .to_jsonld_async(db.as_graph_db_ref())
        .await
        .expect("format");

    // Both books present, title-only; the nested-scope FILTER never binds price.
    assert_eq!(
        rows,
        json!([["ex:b1", "T1", null], ["ex:b2", "T2", null]]),
        "the FILTER inside the nested OPTIONAL scope must not see ?title, so no \
         price is bound; got {rows}"
    );
}
