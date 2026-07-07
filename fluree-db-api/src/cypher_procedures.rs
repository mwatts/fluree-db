//! Neo4j `db.*` / `dbms.*` procedure shims for Cypher clients.
//!
//! Graph tooling (Neo4j Browser, LangChain, driver smoke tests) introspects
//! the database through built-in procedures — `CALL db.labels() YIELD label`
//! and friends — before it issues real queries. Fluree answers the common
//! ones from ledger statistics instead of executing a scan: a
//! [`ProcedureCall`](fluree_db_cypher::ast::ProcedureCall) statement is
//! rewritten here into a constant-rows [`Query`](fluree_db_cypher::ast::Query)
//! AST (an `InlineRows` VALUES source plus the caller's YIELD/WHERE/RETURN),
//! which then lowers and executes through the ordinary read pipeline — so
//! projection, filtering, ordering, and result formatting all behave exactly
//! like any other Cypher read.
//!
//! Answers come from the HEAD-index stats merged with novelty
//! ([`assemble_fast_stats`]), so labels and types written since the last
//! index build are visible. Like Neo4j's own catalog procedures, the
//! answers are lenient about tombstones: a label or key whose every fact
//! was later retracted may keep appearing until a reindex.
//!
//! Supported: `db.labels`, `db.relationshipTypes`, `db.propertyKeys`,
//! `db.schema.visualization` (best effort), `dbms.components`.

use std::collections::{BTreeSet, HashMap};

use fluree_db_core::{is_rdf_type, IndexStats, LedgerSnapshot, OverlayProvider, Sid, ValueTypeTag};
use fluree_db_cypher::ast::{
    Expr, Literal, ProcedureCall, ProjectionItem, Query, ReadClause, ReturnClause, Variable,
    WithClause,
};
use fluree_db_novelty::{assemble_fast_stats, Novelty};

use crate::error::ApiError;
use crate::Result;

/// The Neo4j version this server impersonates for compatibility probes.
// SYNC: keep in step with `server_agent()` in fluree-db-server/src/bolt.rs —
// drivers and Browser feature-gate on the version from both surfaces.
const COMPAT_NEO4J_VERSION: &str = "5.4.0";

/// Rewrite a standalone `CALL proc(…) [YIELD …] [RETURN …]` statement into a
/// constant-rows read query answering the procedure from ledger stats.
///
/// `vocab` / `overrides` are the ledger-context IRI mappings (the same ones
/// the lowering applies to bare identifiers) — label/type/key names are
/// rendered back through them so `db.labels()` returns the identifiers a
/// user would actually write in a MATCH.
pub(crate) fn procedure_call_query(
    call: &ProcedureCall,
    snapshot: &LedgerSnapshot,
    overlay: Option<&dyn OverlayProvider>,
    vocab: Option<&str>,
    overrides: &HashMap<String, String>,
) -> Result<Query> {
    let span = call.span;
    let lit = |s: &str| Expr::Lit(Literal::String(s.to_string(), span));
    let name = call.name.to_ascii_lowercase();

    let (columns, rows): (&[&str], Vec<Vec<Expr>>) = match name.as_str() {
        "db.labels" => {
            require_no_args(call)?;
            let names = schema_names(snapshot, overlay, vocab, overrides);
            (
                &["label"],
                names.labels.iter().map(|l| vec![lit(l)]).collect(),
            )
        }
        "db.relationshiptypes" => {
            require_no_args(call)?;
            let names = schema_names(snapshot, overlay, vocab, overrides);
            (
                &["relationshipType"],
                names.rel_types.iter().map(|t| vec![lit(t)]).collect(),
            )
        }
        "db.propertykeys" => {
            require_no_args(call)?;
            let names = schema_names(snapshot, overlay, vocab, overrides);
            (
                &["propertyKey"],
                names.prop_keys.iter().map(|k| vec![lit(k)]).collect(),
            )
        }
        "db.schema.visualization" => {
            require_no_args(call)?;
            let names = schema_names(snapshot, overlay, vocab, overrides);
            let nodes = Expr::List(
                names
                    .labels
                    .iter()
                    .map(|l| {
                        Expr::Map(
                            vec![
                                ("name".to_string(), lit(l)),
                                ("labels".to_string(), Expr::List(vec![lit(l)], span)),
                            ],
                            span,
                        )
                    })
                    .collect(),
                span,
            );
            let relationships = Expr::List(
                names
                    .rel_types
                    .iter()
                    .map(|t| Expr::Map(vec![("name".to_string(), lit(t))], span))
                    .collect(),
                span,
            );
            (
                &["nodes", "relationships"],
                vec![vec![nodes, relationships]],
            )
        }
        "dbms.components" => {
            require_no_args(call)?;
            // Mirrors the Bolt handshake's `Neo4j/<semver> (compatible; …)`
            // convention: a Neo4j-parseable identity with Fluree attribution.
            let edition = format!(
                "community (compatible; Fluree/{})",
                env!("CARGO_PKG_VERSION")
            );
            (
                &["name", "versions", "edition"],
                vec![vec![
                    lit("Neo4j Kernel"),
                    Expr::List(vec![lit(COMPAT_NEO4J_VERSION)], span),
                    Expr::Lit(Literal::String(edition, span)),
                ]],
            )
        }
        _ => {
            return Err(ApiError::cypher(
                format!(
                    "unknown procedure `{}` — supported: db.labels, db.relationshipTypes, \
                     db.propertyKeys, db.schema.visualization, dbms.components",
                    call.name
                ),
                Vec::new(),
            ))
        }
    };

    build_query(call, columns, rows)
}

fn require_no_args(call: &ProcedureCall) -> Result<()> {
    if call.args.is_empty() {
        Ok(())
    } else {
        Err(ApiError::cypher(
            format!("procedure `{}` takes no arguments", call.name),
            Vec::new(),
        ))
    }
}

/// Sorted display names for the ledger's labels (classes), relationship
/// types (ref-object predicates), and property keys (literal-object
/// predicates), from novelty-merged index stats.
struct SchemaNames {
    labels: BTreeSet<String>,
    rel_types: BTreeSet<String>,
    prop_keys: BTreeSet<String>,
}

fn schema_names(
    snapshot: &LedgerSnapshot,
    overlay: Option<&dyn OverlayProvider>,
    vocab: Option<&str>,
    overrides: &HashMap<String, String>,
) -> SchemaNames {
    let stats = merged_stats(snapshot, overlay);
    // Reverse the ledger-context term overrides so IRIs render back to the
    // short names a Cypher user writes.
    let reverse_overrides: HashMap<&str, &str> = overrides
        .iter()
        .map(|(short, iri)| (iri.as_str(), short.as_str()))
        .collect();
    let display = |sid: &Sid| -> Option<String> {
        let prefix = snapshot.namespaces().get(&sid.namespace_code)?;
        let iri = format!("{}{}", prefix, sid.name);
        // System vocabulary (commit metadata, edge-annotation reifiers, …)
        // is not part of the user's property graph.
        if iri.starts_with("https://ns.flur.ee/") {
            return None;
        }
        if let Some(short) = reverse_overrides.get(iri.as_str()) {
            return Some((*short).to_string());
        }
        if let Some(rest) = vocab.and_then(|v| iri.strip_prefix(v)) {
            if !rest.is_empty() {
                return Some(rest.to_string());
            }
        }
        Some(iri)
    };

    let mut names = SchemaNames {
        labels: BTreeSet::new(),
        rel_types: BTreeSet::new(),
        prop_keys: BTreeSet::new(),
    };

    for entry in stats.classes.iter().flatten() {
        if entry.count == 0 {
            continue;
        }
        if let Some(name) = display(&entry.class_sid) {
            names.labels.insert(name);
        }
    }

    let ref_tag = ValueTypeTag::JSON_LD_ID.as_u8();
    for entry in stats.properties.iter().flatten() {
        if entry.count == 0 {
            continue;
        }
        let sid = Sid::new(entry.sid.0, &entry.sid.1);
        if is_rdf_type(&sid) {
            continue;
        }
        let Some(name) = display(&sid) else { continue };
        let has_ref = entry
            .datatypes
            .iter()
            .any(|&(dt, c)| dt == ref_tag && c > 0);
        let has_literal = entry
            .datatypes
            .iter()
            .any(|&(dt, c)| dt != ref_tag && c > 0);
        if has_ref {
            names.rel_types.insert(name.clone());
        }
        // No datatype breakdown (older index formats) counts as a property
        // key — the common case, and the safer default for graph viz.
        if has_literal || entry.datatypes.is_empty() {
            names.prop_keys.insert(name);
        }
    }

    names
}

/// HEAD-index stats merged with novelty, so labels/types/keys written since
/// the last index build are visible. Non-`Novelty` overlays (policy views)
/// contribute no new schema, so the indexed stats stand alone there.
fn merged_stats(snapshot: &LedgerSnapshot, overlay: Option<&dyn OverlayProvider>) -> IndexStats {
    let indexed = snapshot.stats.clone().unwrap_or_default();
    match overlay.and_then(|o| o.as_any().downcast_ref::<Novelty>()) {
        Some(novelty) => assemble_fast_stats(&indexed, snapshot, novelty, i64::MAX, None),
        None => indexed,
    }
}

/// Assemble the constant-rows query: `InlineRows` over the procedure's
/// columns, a `WITH` realizing YIELD projection/renames and its WHERE, and
/// the caller's RETURN (or an implicit RETURN of the visible columns).
fn build_query(call: &ProcedureCall, columns: &[&str], rows: Vec<Vec<Expr>>) -> Result<Query> {
    let span = call.span;
    let var = |name: &str| Variable {
        name: name.to_string(),
        span,
    };

    for y in &call.yields {
        if !columns.contains(&y.column.as_str()) {
            return Err(ApiError::cypher(
                format!(
                    "unknown YIELD column `{}` for `{}` (columns: {})",
                    y.column,
                    call.name,
                    columns.join(", ")
                ),
                Vec::new(),
            ));
        }
    }

    let mut clauses = vec![ReadClause::InlineRows {
        vars: columns.iter().map(|c| var(c)).collect(),
        rows,
    }];

    // Names visible after YIELD (aliases applied); all columns when bare.
    let visible: Vec<String> = if call.yields.is_empty() {
        columns.iter().map(|c| (*c).to_string()).collect()
    } else {
        call.yields
            .iter()
            .map(|y| {
                y.alias
                    .as_ref()
                    .map_or_else(|| y.column.clone(), |a| a.name.clone())
            })
            .collect()
    };

    if !call.yields.is_empty() || call.where_clause.is_some() {
        let items = if call.yields.is_empty() {
            columns
                .iter()
                .map(|c| ProjectionItem {
                    expr: Expr::Var(var(c)),
                    alias: None,
                    span,
                })
                .collect()
        } else {
            call.yields
                .iter()
                .map(|y| ProjectionItem {
                    expr: Expr::Var(var(&y.column)),
                    alias: y.alias.clone(),
                    span,
                })
                .collect()
        };
        clauses.push(ReadClause::With(WithClause {
            items,
            distinct: false,
            where_clause: call.where_clause.clone(),
            order_by: Vec::new(),
            skip: None,
            limit: None,
            span,
        }));
    }

    let return_clause = call.return_clause.clone().unwrap_or_else(|| ReturnClause {
        items: visible
            .iter()
            .map(|name| ProjectionItem {
                expr: Expr::Var(var(name)),
                alias: None,
                span,
            })
            .collect(),
        distinct: false,
        order_by: Vec::new(),
        skip: None,
        limit: None,
        span,
    });

    Ok(Query {
        clauses,
        return_clause,
        union_tail: None,
        span,
    })
}
