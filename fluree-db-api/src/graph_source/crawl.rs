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

/// Distinct object-var name for the i-th explicit predicate of a predicate-list
/// crawl. A DISTINCT var per predicate is REQUIRED: a shared object var makes
/// the star members a self-join constraint rather than a star, defeating the
/// single-scan star collapse (see `r2rml::rewrite`).
fn crawl_obj_var(i: usize) -> String {
    format!("?__crawl_o{i}")
}

/// The projection shape of a recognized subgraph crawl.
#[derive(Debug, Clone)]
enum CrawlProjection {
    /// `["*"]` — every predicate/object of the subject plus its declared classes.
    Wildcard,
    /// `["@id"]` — subject IRIs only; no property or type scan (the cheapest
    /// shape: it never materializes a predicate-object map).
    IdOnly,
    /// An explicit forward-predicate list (`["v:p1", "v:p2", ...]`). `@id` is
    /// always emitted; `want_type` records an explicit `"@type"` in the list.
    /// Each predicate scans with a DISTINCT object var so the members star-
    /// collapse into ONE scan (and inherit class fusion when the WHERE binds a
    /// class).
    Predicates {
        predicates: Vec<String>,
        want_type: bool,
    },
}

/// A recognized crawl decomposed into the parts the flat-query builder needs.
struct DetectedCrawl<'a> {
    /// The subject variable (e.g. `"?s"`).
    subject_var: &'a str,
    /// The original WHERE clause (binds/filters `?s`).
    where_clause: &'a JsonValue,
    /// The query's `@context`, if any (carried onto the flat query).
    context: Option<&'a JsonValue>,
    /// The crawl's subject LIMIT, if any.
    limit: Option<usize>,
    /// Which projection shape this crawl requests.
    projection: CrawlProjection,
}

/// Accumulates one subject's properties in first-seen order.
struct SubjectAcc {
    /// Distinct class IRIs (`@type`), in first-seen order.
    types: Vec<String>,
    /// `(predicate IRI, values)` pairs, in first-seen order; values de-duplicated.
    props: Vec<(String, Vec<JsonValue>)>,
}

impl SubjectAcc {
    fn empty() -> Self {
        Self {
            types: Vec::new(),
            props: Vec::new(),
        }
    }

    fn add_type(&mut self, type_iri: String) {
        if !self.types.contains(&type_iri) {
            self.types.push(type_iri);
        }
    }

    fn add_value(&mut self, pred: String, value: JsonValue) {
        match self.props.iter_mut().find(|(k, _)| *k == pred) {
            Some((_, vals)) => {
                if !vals.contains(&value) {
                    vals.push(value);
                }
            }
            None => self.props.push((pred, vec![value])),
        }
    }
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
    let Some(DetectedCrawl {
        subject_var,
        where_clause,
        context,
        limit,
        projection,
    }) = detect_wildcard_crawl(input)
    else {
        return Ok(None);
    };

    // Rewrite the crawl into a flat scan: keep the original WHERE (it binds and
    // filters `?s`), then project the columns the projection needs. The flat scan
    // is LIMITed to bound work (an unbounded multi-scan join over a remote table
    // does not early-terminate); the subject LIMIT is re-applied exactly after
    // grouping.
    let flat_limit =
        limit.map(|n| (n.saturating_add(1)).saturating_mul(TRIPLES_PER_SUBJECT_BUDGET));
    let flat_query = build_flat_query(subject_var, where_clause, context, flat_limit, &projection);

    let result = fluree
        .query_view_with_r2rml_options(
            view,
            QueryInput::JsonLd(&flat_query),
            provider,
            table_provider,
            execution,
        )
        .await?;

    let Some(cols) = result.output.columns() else {
        return Ok(None);
    };
    let var_at = |i: usize| -> Option<VarId> {
        match cols.get(i) {
            Some(Column::Var(v)) => Some(*v),
            _ => None,
        }
    };
    // Column 0 is the subject in every crawl's select.
    let Some(s_var) = var_at(0) else {
        return Ok(None);
    };

    let compactor = IriCompactor::new(view.snapshot.shared_namespaces(), &result.context);

    // Group flat rows by subject IRI, preserving first-seen subject order. Type
    // IRIs and property keys are stored already-compacted; `@id` is compacted at
    // assembly time from the raw subject key.
    let mut order: Vec<String> = Vec::new();
    let mut subjects: HashMap<String, SubjectAcc> = HashMap::new();

    match &projection {
        CrawlProjection::IdOnly => {
            for batch in &result.batches {
                for row in 0..batch.len() {
                    let Some(subject_iri) = batch.get(row, s_var).and_then(Binding::get_iri) else {
                        continue;
                    };
                    let key = subject_iri.to_string();
                    subjects.entry(key.clone()).or_insert_with(|| {
                        order.push(key);
                        SubjectAcc::empty()
                    });
                }
            }
        }
        CrawlProjection::Wildcard => {
            // Columns: [?s, ?p, ?o, ?type].
            let (Some(p_var), Some(o_var), Some(t_var)) = (var_at(1), var_at(2), var_at(3)) else {
                return Ok(None);
            };
            for batch in &result.batches {
                for row in 0..batch.len() {
                    let Some(subject_iri) = batch.get(row, s_var).and_then(Binding::get_iri) else {
                        continue;
                    };
                    let key = subject_iri.to_string();
                    let acc = subjects.entry(key.clone()).or_insert_with(|| {
                        order.push(key);
                        SubjectAcc::empty()
                    });
                    if let Some(type_iri) = batch.get(row, t_var).and_then(Binding::get_iri) {
                        acc.add_type(compactor.compact_vocab_iri(type_iri));
                    }
                    if let Some(pred_iri) = batch.get(row, p_var).and_then(Binding::get_iri) {
                        if let Some(obj_binding) = batch.get(row, o_var) {
                            let value =
                                format_binding_with_result(&result, obj_binding, &compactor)?;
                            acc.add_value(compactor.compact_vocab_iri(pred_iri), value);
                        }
                    }
                }
            }
        }
        CrawlProjection::Predicates {
            predicates,
            want_type,
        } => {
            // Columns: [?s, ?__crawl_o0, .., ?__crawl_o{n-1}, (?__crawl_type)?].
            let obj_vars: Vec<Option<VarId>> =
                (0..predicates.len()).map(|i| var_at(i + 1)).collect();
            let type_var = if *want_type {
                var_at(predicates.len() + 1)
            } else {
                None
            };
            for batch in &result.batches {
                for row in 0..batch.len() {
                    let Some(subject_iri) = batch.get(row, s_var).and_then(Binding::get_iri) else {
                        continue;
                    };
                    let key = subject_iri.to_string();
                    let acc = subjects.entry(key.clone()).or_insert_with(|| {
                        order.push(key);
                        SubjectAcc::empty()
                    });
                    for (i, ovar) in obj_vars.iter().enumerate() {
                        let Some(ovar) = ovar else { continue };
                        if let Some(obj_binding) = batch.get(row, *ovar) {
                            if !matches!(obj_binding, Binding::Unbound) {
                                let value =
                                    format_binding_with_result(&result, obj_binding, &compactor)?;
                                acc.add_value(predicates[i].clone(), value);
                            }
                        }
                    }
                    if let Some(tv) = type_var {
                        if let Some(type_iri) = batch.get(row, tv).and_then(Binding::get_iri) {
                            acc.add_type(compactor.compact_vocab_iri(type_iri));
                        }
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
            let types: Vec<JsonValue> = acc.types.into_iter().map(JsonValue::String).collect();
            doc.insert("@type".to_string(), collapse(types, normalize));
        }
        for (pred, values) in acc.props {
            doc.insert(pred, collapse(values, normalize));
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
        // Master switch (default on).
        let expand = env_flag_enabled("FLUREE_R2RML_CRAWL_EXPAND");
        // Coupling: expand-on + class-fusion-off routes a browse through the
        // UNFUSED crawl (a full TriplesMap fan-out + shared-catalog 429 storm —
        // strictly worse than the pre-fix fast empty result). So when the
        // rewriter's class fusion (`FLUREE_R2RML_CRAWL_CLASS_FUSION`) is
        // explicitly disabled, force expansion off too, falling back to native
        // hydration (`[]` for a virtual dataset).
        let class_fusion = env_flag_enabled("FLUREE_R2RML_CRAWL_CLASS_FUSION");
        expand && class_fusion
    })
}

/// Read an on/off environment flag that defaults to **on**. Only `0`, `false`,
/// `off`, or `no` (case-insensitive) disable it.
fn env_flag_enabled(name: &str) -> bool {
    std::env::var(name)
        .map(|v| {
            !matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "off" | "no"
            )
        })
        .unwrap_or(true)
}

/// Cheap check: does `input` look like a subgraph crawl this module expands?
/// Used by the query terminals to skip the (single-ledger) crawl-routing work
/// for ordinary queries. Equivalent to `detect_wildcard_crawl(input).is_some()`.
pub(crate) fn is_wildcard_crawl(input: &JsonValue) -> bool {
    detect_wildcard_crawl(input).is_some()
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

/// Recognize a single-column subgraph crawl `{"select": {"?s": [...]}, ...}` in
/// one of the handled projection shapes (`["*"]`, `["@id"]`, or an explicit
/// predicate list). Returns `None` for any other shape (which then falls back to
/// the normal formatter).
fn detect_wildcard_crawl(input: &JsonValue) -> Option<DetectedCrawl<'_>> {
    let obj = input.as_object()?;
    let select = obj.get("select")?.as_object()?;
    if select.len() != 1 {
        return None;
    }
    let (subject_var, spec) = select.iter().next()?;
    // Only a variable-subject crawl is handled here; a constant-IRI root
    // (`{"<iri>": ["*"]}`) flows through the flat var-predicate path (subject
    // reversal), not this module.
    if !subject_var.starts_with('?') {
        return None;
    }
    let projection = classify_projection(spec.as_array()?)?;
    let where_clause = obj.get("where")?;
    let limit = obj
        .get("limit")
        .and_then(JsonValue::as_u64)
        .map(|n| n as usize);
    Some(DetectedCrawl {
        subject_var: subject_var.as_str(),
        where_clause,
        context: obj.get("@context"),
        limit,
        projection,
    })
}

/// Classify a crawl's selection array into a [`CrawlProjection`]. Returns `None`
/// for shapes this module does not expand (empty, a nested ref-crawl object, or
/// an unsupported JSON-LD keyword), so the caller falls back to normal
/// formatting.
fn classify_projection(spec: &[JsonValue]) -> Option<CrawlProjection> {
    if spec.is_empty() {
        return None;
    }
    // Any `"*"` entry means the full wildcard shape.
    if spec.iter().any(|v| v.as_str() == Some("*")) {
        return Some(CrawlProjection::Wildcard);
    }
    let mut predicates: Vec<String> = Vec::new();
    let mut want_type = false;
    for entry in spec {
        // Only string terms are handled; a nested ref-crawl (object) falls back.
        let key = entry.as_str()?;
        match key {
            "@id" => {} // `@id` is always emitted; it needs no scan.
            "@type" => want_type = true,
            // Any other JSON-LD keyword (`@graph`, ...) isn't a forward
            // predicate — fall back rather than mis-scan.
            _ if key.starts_with('@') => return None,
            _ => predicates.push(key.to_string()),
        }
    }
    if predicates.is_empty() && !want_type {
        // The selection was exactly `["@id"]` (id-only, cheapest).
        Some(CrawlProjection::IdOnly)
    } else {
        Some(CrawlProjection::Predicates {
            predicates,
            want_type,
        })
    }
}

/// Normalize a WHERE clause into a pattern vector (a single-object WHERE is
/// wrapped) so injected scan patterns can be appended.
fn where_as_array(where_clause: &JsonValue) -> Vec<JsonValue> {
    match where_clause {
        JsonValue::Array(patterns) => patterns.clone(),
        other => vec![other.clone()],
    }
}

/// Build the flat scan query for a crawl: the original WHERE (binds/filters
/// `?s`) plus the scan patterns the projection needs, and a matching select.
///
/// - [`CrawlProjection::Wildcard`]: `?s ?p ?o` (every predicate/object) + `?s a
///   ?type` (declared classes).
/// - [`CrawlProjection::IdOnly`]: no injected scan — just project `?s`, so the
///   WHERE's own class/predicate scan binds the subject and nothing else runs.
/// - [`CrawlProjection::Predicates`]: one `?s <p_i> ?__crawl_o{i}` per predicate
///   with a DISTINCT object var (so they star-collapse into one scan), plus an
///   optional `?s a ?__crawl_type`.
fn build_flat_query(
    subject_var: &str,
    where_clause: &JsonValue,
    context: Option<&JsonValue>,
    flat_limit: Option<usize>,
    projection: &CrawlProjection,
) -> JsonValue {
    let mut where_patterns = where_as_array(where_clause);
    let select: Vec<JsonValue> = match projection {
        CrawlProjection::Wildcard => {
            // `?s ?__crawl_p ?__crawl_o` — every (predicate, object) of `?s`.
            where_patterns.push(json!({ "@id": subject_var, CRAWL_PRED: CRAWL_OBJ }));
            // `?s a ?__crawl_type` — the subject's declared class(es).
            where_patterns.push(json!({ "@id": subject_var, "@type": CRAWL_TYPE }));
            vec![
                json!(subject_var),
                json!(CRAWL_PRED),
                json!(CRAWL_OBJ),
                json!(CRAWL_TYPE),
            ]
        }
        CrawlProjection::IdOnly => vec![json!(subject_var)],
        CrawlProjection::Predicates {
            predicates,
            want_type,
        } => {
            let mut select = vec![json!(subject_var)];
            for (i, pred) in predicates.iter().enumerate() {
                let obj_var = crawl_obj_var(i);
                // Build `{"@id": ?s, "<pred>": "?__crawl_o{i}"}` with the
                // predicate as a dynamic key (json! needs literal keys).
                let mut pat = Map::new();
                pat.insert("@id".to_string(), json!(subject_var));
                pat.insert(pred.clone(), json!(obj_var));
                where_patterns.push(JsonValue::Object(pat));
                select.push(json!(obj_var));
            }
            if *want_type {
                where_patterns.push(json!({ "@id": subject_var, "@type": CRAWL_TYPE }));
                select.push(json!(CRAWL_TYPE));
            }
            select
        }
    };

    let mut query = Map::new();
    if let Some(ctx) = context {
        query.insert("@context".to_string(), ctx.clone());
    }
    query.insert("select".to_string(), JsonValue::Array(select));
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
        let DetectedCrawl {
            subject_var,
            where_clause,
            context,
            limit,
            projection,
        } = detect_wildcard_crawl(&input).expect("wildcard crawl");
        assert_eq!(subject_var, "?s");
        assert_eq!(limit, Some(3));
        assert!(context.is_some());
        assert_eq!(where_clause, &json!({"@id": "?s", "@type": "v:Geography"}));
        assert!(matches!(projection, CrawlProjection::Wildcard));
        assert!(is_wildcard_crawl(&input));
    }

    #[test]
    fn detects_id_only_and_predicate_crawls() {
        // `["@id"]` — id-only (cheapest).
        let id_only = json!({"select": {"?s": ["@id"]}, "where": {"@id": "?s", "@type": "v:C"}});
        let projection = detect_wildcard_crawl(&id_only)
            .expect("id-only crawl")
            .projection;
        assert!(matches!(projection, CrawlProjection::IdOnly));
        assert!(is_wildcard_crawl(&id_only));

        // Explicit forward-predicate list — now a recognized crawl (FIX 4).
        let preds = json!({
            "select": {"?s": ["@id", "v:name", "v:age"]},
            "where": {"@id": "?s", "@type": "v:C"}
        });
        let projection = detect_wildcard_crawl(&preds)
            .expect("predicate crawl")
            .projection;
        match projection {
            CrawlProjection::Predicates {
                predicates,
                want_type,
            } => {
                assert_eq!(predicates, vec!["v:name".to_string(), "v:age".to_string()]);
                assert!(!want_type);
            }
            other => panic!("expected Predicates, got {other:?}"),
        }

        // A predicate list that also asks for `@type`.
        let with_type = json!({"select": {"?s": ["v:name", "@type"]}, "where": {"@id": "?s"}});
        let projection = detect_wildcard_crawl(&with_type)
            .expect("predicate+type crawl")
            .projection;
        assert!(matches!(projection, CrawlProjection::Predicates { want_type, .. } if want_type));
    }

    #[test]
    fn falls_back_for_non_crawl_shapes() {
        // Flat select (select is an array, not a subject→projection map).
        assert!(detect_wildcard_crawl(&json!({"select": ["?s"], "where": {}})).is_none());
        // Multi-column projection.
        assert!(
            detect_wildcard_crawl(&json!({"select": {"?s": ["*"], "?x": ["*"]}, "where": {}}))
                .is_none()
        );
        // Missing where.
        assert!(detect_wildcard_crawl(&json!({"select": {"?s": ["*"]}})).is_none());
        // Constant-IRI root is not handled here (flows through the flat path).
        assert!(detect_wildcard_crawl(&json!({"select": {"ex:s": ["*"]}, "where": {}})).is_none());
        // Empty projection list.
        assert!(detect_wildcard_crawl(&json!({"select": {"?s": []}, "where": {}})).is_none());
        // Unsupported JSON-LD keyword in the list.
        assert!(
            detect_wildcard_crawl(&json!({"select": {"?s": ["@graph"]}, "where": {}})).is_none()
        );
    }

    #[test]
    fn flat_query_injects_wildcard_and_type_scans() {
        let context = json!({"v": "http://example/org/ns"});
        let where_clause = json!({"@id": "?s", "@type": "v:Geography"});
        let flat = build_flat_query(
            "?s",
            &where_clause,
            Some(&context),
            Some(256),
            &CrawlProjection::Wildcard,
        );

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
        let flat = build_flat_query("?s", &where_clause, None, None, &CrawlProjection::Wildcard);
        let patterns = flat["where"].as_array().expect("where array");
        // Original single pattern + injected wildcard + type projection.
        assert_eq!(patterns.len(), 3);
        assert_eq!(patterns[0], json!({"@id": "?s", "v:country": "?c"}));
        assert!(flat.get("@context").is_none());
        assert!(flat.get("limit").is_none());
    }

    #[test]
    fn flat_query_id_only_projects_subject_alone() {
        let where_clause = json!({"@id": "?s", "@type": "v:C"});
        let flat = build_flat_query("?s", &where_clause, None, None, &CrawlProjection::IdOnly);
        // Select is just the subject; no scan patterns injected beyond the WHERE.
        assert_eq!(flat["select"], json!(["?s"]));
        let patterns = flat["where"].as_array().expect("where array");
        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0], where_clause);
    }

    #[test]
    fn flat_query_predicates_uses_distinct_object_vars() {
        let where_clause = json!({"@id": "?s", "@type": "v:C"});
        let projection = CrawlProjection::Predicates {
            predicates: vec!["v:name".to_string(), "v:age".to_string()],
            want_type: true,
        };
        let flat = build_flat_query("?s", &where_clause, None, None, &projection);
        // Select: subject, one distinct object var per predicate, then type.
        assert_eq!(
            flat["select"],
            json!(["?s", "?__crawl_o0", "?__crawl_o1", CRAWL_TYPE])
        );
        let patterns = flat["where"].as_array().expect("where array");
        // original + p0 + p1 + type
        assert_eq!(patterns.len(), 4);
        assert_eq!(patterns[1], json!({"@id": "?s", "v:name": "?__crawl_o0"}));
        assert_eq!(patterns[2], json!({"@id": "?s", "v:age": "?__crawl_o1"}));
        assert_eq!(patterns[3], json!({"@id": "?s", "@type": CRAWL_TYPE}));
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

/// End-to-end crawl tests driving the FULL crawl (build flat query → R2RML
/// operator + rewrite fusion → group → JSON-LD docs) against an in-crate mock
/// R2RML provider — no live catalog. Exercises FIX 1/2/4 together: routing,
/// class-fusion scan pruning, the vertical-partition guard, and the id-only /
/// limit / multi-class shapes.
#[cfg(test)]
mod e2e {
    use super::*;
    use async_trait::async_trait;
    use fluree_db_iceberg::io::batch::{BatchSchema, Column, ColumnBatch, FieldInfo, FieldType};
    use fluree_db_query::error::Result as QueryResult;
    use fluree_db_query::r2rml::{
        ColumnBatchStream, R2rmlProvider, R2rmlTableProvider, ScanFilter,
    };
    use fluree_db_r2rml::mapping::{
        CompiledR2rmlMapping, ObjectMap, PredicateMap, PredicateObjectMap, TriplesMap,
    };
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use crate::{FlureeBuilder, LedgerState, Novelty};
    use fluree_db_core::LedgerSnapshot;

    /// Mock provider: one compiled mapping + per-table batches, recording every
    /// scanned table name so tests can assert TriplesMap fan-out was pruned.
    #[derive(Debug)]
    struct MockCrawlProvider {
        mapping: Arc<CompiledR2rmlMapping>,
        tables: HashMap<String, Vec<ColumnBatch>>,
        scanned: Mutex<Vec<String>>,
    }

    impl MockCrawlProvider {
        fn new(mapping: CompiledR2rmlMapping, tables: HashMap<String, Vec<ColumnBatch>>) -> Self {
            Self {
                mapping: Arc::new(mapping),
                tables,
                scanned: Mutex::new(Vec::new()),
            }
        }
        fn scanned_tables(&self) -> Vec<String> {
            let mut v = self.scanned.lock().unwrap().clone();
            v.sort();
            v.dedup();
            v
        }
    }

    #[async_trait]
    impl R2rmlProvider for MockCrawlProvider {
        async fn has_r2rml_mapping(&self, _gs: &str) -> bool {
            true
        }
        async fn compiled_mapping(
            &self,
            _gs: &str,
            _as_of_t: Option<i64>,
        ) -> QueryResult<Arc<CompiledR2rmlMapping>> {
            Ok(Arc::clone(&self.mapping))
        }
    }

    #[async_trait]
    impl R2rmlTableProvider for MockCrawlProvider {
        async fn scan_table(
            &self,
            _gs: &str,
            table: &str,
            _projection: &[String],
            _filters: &[ScanFilter],
            _as_of_t: Option<i64>,
        ) -> QueryResult<ColumnBatchStream> {
            self.scanned.lock().unwrap().push(table.to_string());
            let batches = self.tables.get(table).cloned().unwrap_or_default();
            use futures::StreamExt;
            Ok(Box::pin(futures::stream::iter(batches).map(Ok)))
        }
    }

    /// A `TriplesMap`: table + subject template + one class + one string POM.
    fn tm(
        iri: &str,
        table: &str,
        template: &str,
        class: &str,
        pred: &str,
        col: &str,
    ) -> TriplesMap {
        TriplesMap::new(iri, table)
            .with_subject_template(template)
            .with_class(class)
            .with_predicate_object(PredicateObjectMap {
                predicate_map: PredicateMap::constant(pred),
                object_map: ObjectMap::column(col),
            })
    }

    /// One batch with an `id` (Int64) column and one nullable String column.
    fn id_str_batch(col: &str, ids: &[i64], vals: &[&str]) -> ColumnBatch {
        let schema = BatchSchema::new(vec![
            FieldInfo {
                name: "id".to_string(),
                field_type: FieldType::Int64,
                nullable: false,
                field_id: 1,
            },
            FieldInfo {
                name: col.to_string(),
                field_type: FieldType::String,
                nullable: true,
                field_id: 2,
            },
        ]);
        ColumnBatch::new(
            Arc::new(schema),
            vec![
                Column::Int64(ids.iter().map(|i| Some(*i)).collect()),
                Column::String(vals.iter().map(|s| Some((*s).to_string())).collect()),
            ],
        )
        .unwrap()
    }

    /// A genesis graph-source view with the `example.org` namespace registered.
    /// Returns the backing ledger too so its snapshot Arc stays alive.
    fn genesis_view() -> (LedgerState, GraphDb) {
        let snapshot = LedgerSnapshot::genesis("crawl-e2e:main");
        let ledger = LedgerState::new(snapshot, Novelty::new(0));
        let mut view = GraphDb::from_ledger_state(&ledger);
        Arc::make_mut(&mut view.snapshot)
            .insert_namespace_code(9_999, "http://example.org/".to_string())
            .unwrap();
        view.graph_source_id = Some("crawl-e2e:main".into());
        (ledger, view)
    }

    async fn run_crawl(
        provider: &MockCrawlProvider,
        view: &GraphDb,
        crawl: &JsonValue,
    ) -> Vec<JsonValue> {
        let fluree = FlureeBuilder::memory().build_memory();
        expand_wildcard_crawl(
            &fluree,
            view,
            crawl,
            provider,
            provider,
            QueryExecutionOptions::new(),
            &FormatterConfig::default(),
        )
        .await
        .expect("crawl expansion succeeds")
        .expect("crawl shape is handled")
        .as_array()
        .expect("crawl returns a JSON array")
        .clone()
    }

    fn two_table_provider() -> MockCrawlProvider {
        let mapping = CompiledR2rmlMapping::new(vec![
            tm(
                "#People",
                "people",
                "http://example.org/person/{id}",
                "http://example.org/Person",
                "http://example.org/name",
                "name",
            ),
            tm(
                "#Orders",
                "orders",
                "http://example.org/order/{id}",
                "http://example.org/Order",
                "http://example.org/label",
                "label",
            ),
        ]);
        let mut tables = HashMap::new();
        tables.insert(
            "people".to_string(),
            vec![id_str_batch("name", &[1, 2], &["Alice", "Bob"])],
        );
        tables.insert(
            "orders".to_string(),
            vec![id_str_batch("label", &[10, 11], &["O-10", "O-11"])],
        );
        MockCrawlProvider::new(mapping, tables)
    }

    fn person_crawl(projection: JsonValue, limit: Option<u64>) -> JsonValue {
        let mut q = serde_json::Map::new();
        q.insert("@context".into(), json!({"v": "http://example.org/"}));
        q.insert("select".into(), json!({"?s": projection}));
        q.insert("where".into(), json!({"@id": "?s", "@type": "v:Person"}));
        if let Some(n) = limit {
            q.insert("limit".into(), json!(n));
        }
        JsonValue::Object(q)
    }

    fn ids(docs: &[JsonValue]) -> std::collections::BTreeSet<String> {
        docs.iter()
            .filter_map(|d| d.get("@id").and_then(|v| v.as_str()).map(str::to_string))
            .collect()
    }

    // (a) A wildcard `["*"]` crawl returns the SAME subjects as an `["@id"]` crawl.
    #[tokio::test]
    async fn crawl_wildcard_subjects_match_id_only() {
        let provider = two_table_provider();
        let (_ledger, view) = genesis_view();
        let wildcard = run_crawl(&provider, &view, &person_crawl(json!(["*"]), None)).await;
        let id_only = run_crawl(&provider, &view, &person_crawl(json!(["@id"]), None)).await;
        assert_eq!(wildcard.len(), 2, "two Person instances");
        assert_eq!(ids(&wildcard), ids(&id_only), "same subject set both ways");
        assert!(
            ids(&wildcard).iter().all(|s| !s.contains("order")),
            "only Person (people) subjects, never Order subjects"
        );
    }

    // (b) `["@id"]` returns ids (each doc is exactly `{"@id": ...}`), not `[]`.
    #[tokio::test]
    async fn crawl_id_only_returns_ids_not_empty() {
        let provider = two_table_provider();
        let (_ledger, view) = genesis_view();
        let docs = run_crawl(&provider, &view, &person_crawl(json!(["@id"]), None)).await;
        assert!(!docs.is_empty(), "id-only crawl must return ids, not []");
        for d in &docs {
            let obj = d.as_object().expect("doc is an object");
            assert!(obj.contains_key("@id"));
            assert_eq!(obj.len(), 1, "id-only doc carries @id only: {obj:?}");
        }
    }

    // (c) A one-class `["*"]` crawl over a multi-TriplesMap mapping scans ONLY the
    //     queried class's table (fusion prunes the fan-out).
    #[tokio::test]
    async fn crawl_wildcard_scans_only_class_table() {
        let provider = two_table_provider();
        let (_ledger, view) = genesis_view();
        let _ = run_crawl(&provider, &view, &person_crawl(json!(["*"]), None)).await;
        assert_eq!(
            provider.scanned_tables(),
            vec!["people".to_string()],
            "class fusion must prune the scan to the Person table only"
        );
    }

    // (d) A 2nd TriplesMap sharing the subject template but lacking the class
    //     forces the guard to REFUSE fusion, so the wildcard still returns that
    //     map's triples (no silent under-fetch).
    #[tokio::test]
    async fn crawl_wildcard_vertical_partition_returns_second_map() {
        let mapping = CompiledR2rmlMapping::new(vec![
            tm(
                "#PersonClass",
                "people",
                "http://example.org/person/{id}",
                "http://example.org/Person",
                "http://example.org/name",
                "name",
            ),
            // Same subject template, NO class, a distinct predicate/table.
            TriplesMap::new("#PersonEmail", "people_email")
                .with_subject_template("http://example.org/person/{id}")
                .with_predicate_object(PredicateObjectMap {
                    predicate_map: PredicateMap::constant("http://example.org/email"),
                    object_map: ObjectMap::column("email"),
                }),
        ]);
        let mut tables = HashMap::new();
        tables.insert(
            "people".to_string(),
            vec![id_str_batch("name", &[1], &["Alice"])],
        );
        tables.insert(
            "people_email".to_string(),
            vec![id_str_batch("email", &[1], &["alice@example.org"])],
        );
        let provider = MockCrawlProvider::new(mapping, tables);
        let (_ledger, view) = genesis_view();
        let docs = run_crawl(&provider, &view, &person_crawl(json!(["*"]), None)).await;
        assert_eq!(docs.len(), 1);
        let serialized = serde_json::to_string(&docs).unwrap();
        assert!(
            serialized.contains("alice@example.org"),
            "vertical-partition guard must keep the classless map's email triple: {serialized}"
        );
        assert!(
            provider
                .scanned_tables()
                .contains(&"people_email".to_string()),
            "the classless second table must still be scanned"
        );
    }

    // (e) A multi-class subject's `@type` includes ALL declared classes.
    #[tokio::test]
    async fn crawl_wildcard_multi_class_type_complete() {
        let mapping = CompiledR2rmlMapping::new(vec![TriplesMap::new("#PA", "people")
            .with_subject_template("http://example.org/person/{id}")
            .with_class("http://example.org/Person")
            .with_class("http://example.org/Agent")
            .with_predicate_object(PredicateObjectMap {
                predicate_map: PredicateMap::constant("http://example.org/name"),
                object_map: ObjectMap::column("name"),
            })]);
        let mut tables = HashMap::new();
        tables.insert(
            "people".to_string(),
            vec![id_str_batch("name", &[1], &["Alice"])],
        );
        let provider = MockCrawlProvider::new(mapping, tables);
        let (_ledger, view) = genesis_view();
        let docs = run_crawl(&provider, &view, &person_crawl(json!(["*"]), None)).await;
        assert_eq!(docs.len(), 1);
        let types = docs[0].get("@type").expect("has @type");
        let type_list = types
            .as_array()
            .cloned()
            .unwrap_or_else(|| vec![types.clone()]);
        assert_eq!(
            type_list.len(),
            2,
            "class-constrained type-var must bind BOTH declared classes: {type_list:?}"
        );
    }

    // (f) A LIMIT k crawl returns exactly k subjects.
    #[tokio::test]
    async fn crawl_wildcard_limit_returns_exactly_k() {
        let mapping = CompiledR2rmlMapping::new(vec![tm(
            "#People",
            "people",
            "http://example.org/person/{id}",
            "http://example.org/Person",
            "http://example.org/name",
            "name",
        )]);
        let mut tables = HashMap::new();
        tables.insert(
            "people".to_string(),
            vec![id_str_batch("name", &[1, 2, 3], &["Alice", "Bob", "Cara"])],
        );
        let provider = MockCrawlProvider::new(mapping, tables);
        let (_ledger, view) = genesis_view();
        let docs = run_crawl(&provider, &view, &person_crawl(json!(["*"]), Some(2))).await;
        assert_eq!(docs.len(), 2, "LIMIT 2 must return exactly 2 subjects");
    }
}
