//! Graph-source (R2RML/Iceberg) subgraph-crawl hydration.
//!
//! FQL's subgraph / "crawl" projection (`{"select": {"?s": ["*"]}}`) is normally
//! satisfied by the async hydration formatter, which fetches each bound subject's
//! flakes from the native binary index. An R2RML graph source has **no**
//! binary-index flakes — its data lives in Iceberg and is only reachable through
//! the R2RML operator — so native hydration resolves every subject to `null` and
//! the crawl returns an empty array (`[]`), which is what makes the Solo
//! virtual-dataset "View Instances" screen come back empty.
//!
//! This module expands a wildcard crawl over a graph source through the R2RML
//! operator instead: it rewrites the crawl into a flat wildcard scan
//! (`?s ?p ?o` + `?s a ?type`, which the operator now binds — see
//! `fluree_db_query::r2rml` `predicate_var` / `type_var`), executes it via the
//! same R2RML query path the rest of the engine uses, and regroups the flat
//! `(subject, predicate, object, type)` rows into per-subject JSON-LD documents.
//!
//! Scope: only a **wildcard** (`["*"]`) single-column crawl is expanded here.
//! Explicit-predicate selections, nested ref-crawls, and multi-column projections
//! fall back to the normal path (returning `Ok(None)`), so this never changes the
//! behavior of any query it does not fully handle.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde_json::{json, Map, Value as JsonValue};

use fluree_db_query::ir::projection::Column;
use fluree_db_query::var_registry::VarId;
use fluree_db_query::Binding;

use crate::format::{format_binding_with_result, FormatterConfig, IriCompactor};
use crate::view::{GraphDb, QueryInput};
use crate::{Fluree, QueryExecutionOptions, Result};

/// Fresh variable names for the injected wildcard scan. The leading `?__` keeps
/// them from colliding with any user variable.
const CRAWL_PRED: &str = "?__crawl_p";
const CRAWL_OBJ: &str = "?__crawl_o";
const CRAWL_TYPE: &str = "?__crawl_type";

/// Per-subject triple budget used to translate the crawl's **subject** LIMIT into
/// the flat query's **triple** LIMIT. The flat scan fetches `(limit + 1) × BUDGET`
/// triples — enough that the first `limit` subjects are fully materialized (each
/// with up to `BUDGET` predicate/object/type triples) while still bounding the
/// scan so it early-terminates instead of walking the whole table. A subject with
/// more than `BUDGET` triples may be truncated (acceptable for the tabular
/// dimension/fact tables R2RML maps; rows are wide in columns, not in triples).
const TRIPLES_PER_SUBJECT_BUDGET: usize = 64;

/// Accumulates one subject's properties in first-seen order.
struct SubjectAcc {
    /// Distinct class IRIs (`@type`), in first-seen order.
    types: Vec<String>,
    /// `(predicate IRI, values)` pairs, in first-seen order; values de-duplicated.
    props: Vec<(String, Vec<JsonValue>)>,
}

/// Expand a wildcard subgraph crawl over an R2RML graph source, returning the
/// per-subject JSON-LD documents. Returns `Ok(None)` when `input` is not a
/// wildcard crawl this path handles, so the caller falls back to normal
/// formatting.
pub(crate) async fn expand_wildcard_crawl(
    fluree: &Fluree,
    view: &GraphDb,
    input: &JsonValue,
    provider: &dyn fluree_db_query::r2rml::R2rmlProvider,
    table_provider: &dyn fluree_db_query::r2rml::R2rmlTableProvider,
    execution: QueryExecutionOptions,
    format_config: &FormatterConfig,
) -> Result<Option<JsonValue>> {
    let Some((subject_var, where_clause, context, limit)) = detect_wildcard_crawl(input) else {
        return Ok(None);
    };

    // Rewrite the crawl into a flat wildcard scan: keep the original WHERE (it
    // binds and filters `?s`), then project every predicate/object and class. The
    // flat scan is LIMITed to bound work (an unbounded multi-scan join over a
    // remote table does not early-terminate); the subject LIMIT is re-applied
    // exactly after grouping.
    let flat_limit =
        limit.map(|n| (n.saturating_add(1)).saturating_mul(TRIPLES_PER_SUBJECT_BUDGET));
    let flat_query = build_flat_query(subject_var, where_clause, context, flat_limit);

    let result = fluree
        .query_view_with_r2rml_options(
            view,
            QueryInput::JsonLd(&flat_query),
            provider,
            table_provider,
            execution,
        )
        .await?;

    // Resolve the projection columns by select order: [?s, ?p, ?o, ?type].
    let Some(cols) = result.output.columns() else {
        return Ok(None);
    };
    let var_at = |i: usize| -> Option<VarId> {
        match cols.get(i) {
            Some(Column::Var(v)) => Some(*v),
            _ => None,
        }
    };
    let (Some(s_var), Some(p_var), Some(o_var), Some(t_var)) =
        (var_at(0), var_at(1), var_at(2), var_at(3))
    else {
        return Ok(None);
    };

    let compactor = IriCompactor::new(view.snapshot.shared_namespaces(), &result.context);

    // Group flat rows by subject IRI, preserving first-seen subject order.
    let mut order: Vec<String> = Vec::new();
    let mut subjects: HashMap<String, SubjectAcc> = HashMap::new();
    for batch in &result.batches {
        for row in 0..batch.len() {
            let Some(subject_iri) = batch.get(row, s_var).and_then(Binding::get_iri) else {
                continue;
            };
            let key = subject_iri.to_string();
            let acc = subjects.entry(key.clone()).or_insert_with(|| {
                order.push(key.clone());
                SubjectAcc {
                    types: Vec::new(),
                    props: Vec::new(),
                }
            });

            if let Some(type_iri) = batch.get(row, t_var).and_then(Binding::get_iri) {
                let t = type_iri.to_string();
                if !acc.types.contains(&t) {
                    acc.types.push(t);
                }
            }

            if let Some(pred_iri) = batch.get(row, p_var).and_then(Binding::get_iri) {
                if let Some(obj_binding) = batch.get(row, o_var) {
                    let value = format_binding_with_result(&result, obj_binding, &compactor)?;
                    let pred = pred_iri.to_string();
                    match acc.props.iter_mut().find(|(k, _)| *k == pred) {
                        Some((_, vals)) => {
                            if !vals.contains(&value) {
                                vals.push(value);
                            }
                        }
                        None => acc.props.push((pred, vec![value])),
                    }
                }
            }
        }
    }

    // Assemble per-subject JSON-LD documents, honoring the crawl's subject LIMIT.
    let normalize = format_config.normalize_arrays;
    let take = limit.unwrap_or(usize::MAX);
    let mut docs: Vec<JsonValue> = Vec::new();
    for key in order.into_iter().take(take) {
        let acc = subjects.remove(&key).expect("accumulated subject");
        let mut doc = Map::new();
        doc.insert("@id".to_string(), json!(compactor.compact_id_iri(&key)));
        if !acc.types.is_empty() {
            let types: Vec<JsonValue> = acc
                .types
                .iter()
                .map(|t| json!(compactor.compact_vocab_iri(t)))
                .collect();
            doc.insert("@type".to_string(), collapse(types, normalize));
        }
        for (pred, values) in acc.props {
            doc.insert(
                compactor.compact_vocab_iri(&pred),
                collapse(values, normalize),
            );
        }
        docs.push(JsonValue::Object(doc));
    }

    Ok(Some(JsonValue::Array(docs)))
}

/// Master kill-switch for expanding a subgraph crawl over a graph source through
/// the R2RML operator. Default **on**. Set `FLUREE_R2RML_CRAWL_EXPAND=0` (or
/// `false`/`off`) to restore native binary-index hydration — which returns `[]`
/// for a virtual dataset (the pre-fix behavior), so this is a safety escape
/// hatch, not a normal setting.
fn crawl_expand_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("FLUREE_R2RML_CRAWL_EXPAND")
            .map(|v| {
                !matches!(
                    v.trim().to_ascii_lowercase().as_str(),
                    "0" | "false" | "off" | "no"
                )
            })
            .unwrap_or(true)
    })
}

/// Interception entry point used by **every** formatting terminal (the
/// graph-source alias path *and* the ledger-scoped / dataset / connection paths).
///
/// If `json` is a subgraph "crawl" projection over a graph-source-backed `view`,
/// expand it through the R2RML operator and return the per-subject JSON-LD
/// documents. Returns `Ok(None)` — so the caller falls back to its normal
/// (native) formatting — when: the kill-switch is off, there is no R2RML
/// provider, the input is not JSON-LD, the view is not graph-source-backed
/// (`graph_source_id` is `None`, i.e. a genuinely native ledger), or the crawl
/// shape is not one this module handles. This is what makes the Solo virtual
/// dataset "View Instances" screen return data instead of `[]`.
pub(crate) async fn maybe_expand_crawl(
    fluree: &Fluree,
    view: &GraphDb,
    json: Option<&JsonValue>,
    r2rml: Option<(
        &dyn fluree_db_query::r2rml::R2rmlProvider,
        &dyn fluree_db_query::r2rml::R2rmlTableProvider,
    )>,
    execution: QueryExecutionOptions,
    format_config: &FormatterConfig,
) -> Result<Option<JsonValue>> {
    if !crawl_expand_enabled() {
        return Ok(None);
    }
    // Native ledgers (no graph source) hydrate against their binary index as
    // before — this gate is the load-bearing guard that keeps native crawls,
    // and any non-graph-source view, on their existing path.
    if view.graph_source_id.is_none() {
        return Ok(None);
    }
    let (Some((provider, table_provider)), Some(json)) = (r2rml, json) else {
        return Ok(None);
    };
    expand_wildcard_crawl(
        fluree,
        view,
        json,
        provider,
        table_provider,
        execution,
        format_config,
    )
    .await
}

/// Recognize a single-column wildcard crawl `{"select": {"?s": ["*"]}, ...}`.
/// Returns `(subject_var, where_clause, context, limit)` or `None` for any other
/// shape (which then falls back to the normal formatter).
fn detect_wildcard_crawl(
    input: &JsonValue,
) -> Option<(&str, &JsonValue, Option<&JsonValue>, Option<usize>)> {
    let obj = input.as_object()?;
    let select = obj.get("select")?.as_object()?;
    if select.len() != 1 {
        return None;
    }
    let (subject_var, spec) = select.iter().next()?;
    if !subject_var.starts_with('?') {
        return None;
    }
    // The selection must be exactly a wildcard `["*"]` — explicit predicate lists
    // and nested ref-crawls are handled elsewhere (fall back).
    let spec = spec.as_array()?;
    if spec.is_empty() || spec.iter().any(|v| v.as_str() != Some("*")) {
        return None;
    }
    let where_clause = obj.get("where")?;
    let limit = obj
        .get("limit")
        .and_then(JsonValue::as_u64)
        .map(|n| n as usize);
    Some((
        subject_var.as_str(),
        where_clause,
        obj.get("@context"),
        limit,
    ))
}

/// Build the flat wildcard scan query: the original WHERE (binds/filters `?s`)
/// plus an all-predicates scan and a class projection on `?s`.
fn build_flat_query(
    subject_var: &str,
    where_clause: &JsonValue,
    context: Option<&JsonValue>,
    flat_limit: Option<usize>,
) -> JsonValue {
    let mut where_patterns: Vec<JsonValue> = match where_clause {
        JsonValue::Array(patterns) => patterns.clone(),
        other => vec![other.clone()],
    };
    // `?s ?__crawl_p ?__crawl_o` — every (predicate, object) of the subject.
    where_patterns.push(json!({ "@id": subject_var, CRAWL_PRED: CRAWL_OBJ }));
    // `?s a ?__crawl_type` — the subject's declared class(es).
    where_patterns.push(json!({ "@id": subject_var, "@type": CRAWL_TYPE }));

    let mut query = Map::new();
    if let Some(ctx) = context {
        query.insert("@context".to_string(), ctx.clone());
    }
    query.insert(
        "select".to_string(),
        json!([subject_var, CRAWL_PRED, CRAWL_OBJ, CRAWL_TYPE]),
    );
    query.insert("where".to_string(), JsonValue::Array(where_patterns));
    if let Some(n) = flat_limit {
        query.insert("limit".to_string(), json!(n));
    }
    JsonValue::Object(query)
}

/// A single value renders bare (unless array-normalization is on); multiple
/// values always render as a JSON array.
fn collapse(mut values: Vec<JsonValue>, normalize: bool) -> JsonValue {
    if !normalize && values.len() == 1 {
        values.pop().expect("len == 1")
    } else {
        JsonValue::Array(values)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_wildcard_crawl_and_extracts_parts() {
        let input = json!({
            "@context": {"v": "http://example/org/ns"},
            "select": {"?s": ["*"]},
            "where": {"@id": "?s", "@type": "v:Geography"},
            "limit": 3
        });
        let (subject_var, where_clause, context, limit) =
            detect_wildcard_crawl(&input).expect("wildcard crawl");
        assert_eq!(subject_var, "?s");
        assert_eq!(limit, Some(3));
        assert!(context.is_some());
        assert_eq!(where_clause, &json!({"@id": "?s", "@type": "v:Geography"}));
    }

    #[test]
    fn falls_back_for_non_wildcard_shapes() {
        // Flat select (no crawl).
        assert!(detect_wildcard_crawl(&json!({"select": ["?s"], "where": {}})).is_none());
        // Explicit-predicate crawl (not a bare wildcard) — handled elsewhere.
        assert!(
            detect_wildcard_crawl(&json!({"select": {"?s": ["v:country"]}, "where": {}})).is_none()
        );
        // Multi-column projection.
        assert!(
            detect_wildcard_crawl(&json!({"select": {"?s": ["*"], "?x": ["*"]}, "where": {}}))
                .is_none()
        );
        // Missing where.
        assert!(detect_wildcard_crawl(&json!({"select": {"?s": ["*"]}})).is_none());
    }

    #[test]
    fn flat_query_injects_wildcard_and_type_scans() {
        let context = json!({"v": "http://example/org/ns"});
        let where_clause = json!({"@id": "?s", "@type": "v:Geography"});
        let flat = build_flat_query("?s", &where_clause, Some(&context), Some(256));

        assert_eq!(
            flat["select"],
            json!(["?s", CRAWL_PRED, CRAWL_OBJ, CRAWL_TYPE])
        );
        assert_eq!(flat["@context"], context);
        assert_eq!(flat["limit"], json!(256));
        // where = [ original, wildcard(?s ?p ?o), type-projection(?s a ?type) ]
        let patterns = flat["where"].as_array().expect("where array");
        assert_eq!(patterns.len(), 3);
        assert_eq!(patterns[0], where_clause);
        assert_eq!(patterns[1], json!({"@id": "?s", CRAWL_PRED: CRAWL_OBJ}));
        assert_eq!(patterns[2], json!({"@id": "?s", "@type": CRAWL_TYPE}));
    }

    #[test]
    fn flat_query_wraps_array_where() {
        let where_clause = json!([{"@id": "?s", "v:country": "?c"}]);
        let flat = build_flat_query("?s", &where_clause, None, None);
        let patterns = flat["where"].as_array().expect("where array");
        // Original single pattern + injected wildcard + type projection.
        assert_eq!(patterns.len(), 3);
        assert_eq!(patterns[0], json!({"@id": "?s", "v:country": "?c"}));
        assert!(flat.get("@context").is_none());
        assert!(flat.get("limit").is_none());
    }

    #[test]
    fn collapse_unwraps_single_unless_normalized() {
        assert_eq!(collapse(vec![json!("x")], false), json!("x"));
        assert_eq!(collapse(vec![json!("x")], true), json!(["x"]));
        assert_eq!(
            collapse(vec![json!("x"), json!("y")], false),
            json!(["x", "y"])
        );
    }
}
