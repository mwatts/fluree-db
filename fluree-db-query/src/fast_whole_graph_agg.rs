//! Fast-path: whole-graph scalar aggregates over a distinct-subject scan.
//!
//! Cypher `MATCH (n) RETURN count(n), count(n.age)` (and the min/max/avg/sum
//! family) lowers to:
//!
//! ```text
//! patterns: [ Subquery(SELECT DISTINCT ?n WHERE { ?n ?p ?o }),
//!             Optional([ ?n <P> ?prop ]) ]      -- one accessor, at most
//! grouping: Implicit, aggregates over ?n / ?prop
//! ```
//!
//! Executed generally, the subquery scans every flake in the graph and dedups
//! subjects through the DISTINCT operator before a single aggregate row comes
//! out — linear in graph size. Every aggregate the shape admits is answerable
//! from index directories and predicate-scoped scans instead:
//!
//! - `count(?n)` / `COUNT(*)`: each distinct subject contributes
//!   `max(1, #P(n))` rows through the left join, so the row count is
//!   `N + count(P) − subj(P)` (`N` alone with no accessor) — all three terms
//!   are directory-only reads.
//! - `count(?prop)` counts non-null rows = `count(P)`, the predicate's live
//!   row count. Exact for multi-valued `P` (each value is its own row).
//! - `count(DISTINCT ?n)` = `N`; `count(DISTINCT ?prop)` = the predicate's
//!   distinct-object lead-group count.
//! - `min/max(?prop)` over the left join ignore nulls and duplicates — the
//!   POST boundary-key reduction ([`crate::fast_min_max_string`]).
//! - `avg/sum(?prop)`: the rows carrying `?prop` are exactly one per `P`
//!   value, so the predicate-scoped POST fold
//!   ([`crate::fast_predicate_scalar_agg`]) matches the pipeline.
//!
//! With two or more accessor optionals the per-subject rows multiply
//! (`#P1(n) × #P2(n)`), which changes duplicate-sensitive aggregates — the
//! detector only admits the single-accessor shape.
//!
//! The class-anchored family generalizes the same folds to
//! `MATCH (n:C) RETURN …` (anchor = `?n rdf:type <C>`): the subject universe
//! becomes the class's instance count from the per-graph class stats, and
//! property-derived folds additionally require the **containment proof**
//! `class_property_flakes(C, P) == predicate_rows(P)` — equality means every
//! `P`-bearing subject is a `C`, so predicate-scoped folds equal the
//! class-restricted join. Both sides are current-state-exact at HEAD (class
//! stats per issue #1266; rows from PSOT directories).
//!
//! Beyond the single-row scalar shape, `GROUP BY ?prop` + `COUNT(*)` (the
//! Cypher histogram `RETURN n.age, COUNT(*)`) folds to a predicate-scoped
//! POST run-length group count (values are physically contiguous in POST)
//! plus one null-group row (`subjects − subj(P)` — the left join gives
//! property-less subjects a single unbound-key row).
//!
//! The fold requires the strict metadata-lane gate ([`fast_path_store`]:
//! single-ledger, root/no policy, no overlay, at HEAD). The whole-graph
//! anchor additionally declines when the graph carries predicates the
//! variable-predicate scan hides (`f:reifies*` anywhere, the `f:` namespace
//! in the default graph) — those facts are invisible to `?n ?p ?o` but not
//! to the SPOT directories; a class anchor reads only class stats, so it is
//! immune.

use crate::binding::{Batch, Binding};
use crate::error::{QueryError, Result};
use crate::fast_count::{
    count_distinct_lead_groups, count_distinct_objects_for_predicate,
    count_distinct_subjects_for_predicate,
};
use crate::fast_min_max_string::{minmax_numeric_post, minmax_string_dict_post, MinMaxMode};
use crate::fast_path_common::{
    count_rows_for_predicate_psot, count_to_i64, fast_path_store, normalize_pred_sid,
    FastPathOperator,
};
use crate::fast_predicate_scalar_agg::{scan_predicate_scalar_agg, ScalarAggKind, SumExprI64};
use crate::ir::grouping::{AggregateFn, Aggregation, Grouping, InputSemantics};
use crate::ir::triple::{Ref, Term};
use crate::ir::Expression;
use crate::ir::{Pattern, Query};
use crate::operator::BoxedOperator;
use crate::var_registry::VarId;
use fluree_db_binary_index::format::run_record::RunSortOrder;
use fluree_db_binary_index::BinaryIndexStore;
use fluree_db_core::{FlakeValue, GraphId, Sid};
use std::sync::Arc;

/// One aggregate of the recognized shape, mapped to its fold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AggTask {
    /// `count(?n)` / `COUNT(*)` — total pipeline rows.
    CountRows,
    /// `count(DISTINCT ?n)` — distinct subjects.
    CountSubjects,
    /// `count(?prop)` — the accessor predicate's live row count.
    CountProp,
    /// `count(DISTINCT ?prop)` — the predicate's distinct objects.
    CountDistinctProp,
    MinProp,
    MaxProp,
    AvgProp,
    SumProp,
}

impl AggTask {
    fn needs_accessor(self) -> bool {
        !matches!(self, AggTask::CountRows | AggTask::CountSubjects)
    }
}

/// The subject universe the aggregates range over.
enum Anchor {
    /// Bare `MATCH (n)`: the DISTINCT-subject subquery over `?n ?p ?o`.
    AllSubjects,
    /// `MATCH (n:C)`: `?n rdf:type <C>` with a concrete class SID.
    Class(Sid),
}

/// What the fold produces.
enum FoldKind {
    /// One row of scalar aggregates (implicit grouping), one task per
    /// projected column in projection order.
    Scalars(Vec<AggTask>),
    /// `GROUP BY ?prop` + `COUNT(*)`: one row per distinct property value
    /// plus a null group. Column positions index into `schema`.
    Histogram { prop_col: usize, count_col: usize },
}

/// Detected plan: anchor, accessor predicate (if any), fold kind, and the
/// output schema in projection order.
pub(crate) struct WholeGraphAggPlan {
    anchor: Anchor,
    accessor: Option<Ref>,
    kind: FoldKind,
    schema: Vec<VarId>,
}

/// Recognize the whole-graph / class-anchored aggregate shapes. See the
/// module docs for the exact patterns and per-aggregate soundness arguments.
pub(crate) fn detect_whole_graph_scalar_aggs(query: &Query) -> Option<WholeGraphAggPlan> {
    if !query.ordering.is_empty()
        || !query.order_binds.is_empty()
        || query.offset.is_some()
        || query.output.is_distinct()
        || query.limit == Some(0)
        || query.post_values.is_some()
    {
        return None;
    }

    // Anchor: DISTINCT-subject subquery, or a single class triple.
    let (first, rest) = query.patterns.split_first()?;
    let (anchor, subject_var) = match first {
        Pattern::Subquery(sq) => (Anchor::AllSubjects, distinct_subject_scan_var(sq)?),
        Pattern::Triple(tp) => {
            if !tp.p.is_rdf_type() || tp.dtc.is_some() {
                return None;
            }
            let sv = tp.s.as_var()?;
            let Term::Sid(class_sid) = &tp.o else {
                return None;
            };
            (Anchor::Class(class_sid.clone()), sv)
        }
        _ => return None,
    };

    // At most one single-triple property-accessor OPTIONAL, optionally
    // followed by an identity Bind aliasing the accessor var into the
    // projected column (`RETURN n.age, COUNT(*)` emits `?alias = ?prop`).
    let parse_accessor = |inner: &[Pattern]| -> Option<(Ref, VarId)> {
        let [Pattern::Triple(tp)] = inner else {
            return None;
        };
        if tp.s.as_var() != Some(subject_var) || !tp.p_bound() || tp.dtc.is_some() {
            return None;
        }
        let Term::Var(prop_var) = tp.o else {
            return None;
        };
        if prop_var == subject_var {
            return None;
        }
        Some((tp.p.clone(), prop_var))
    };
    let (accessor, prop_alias) = match rest {
        [] => (None, None),
        [Pattern::Optional(inner)] => (Some(parse_accessor(inner)?), None),
        [Pattern::Optional(inner), Pattern::Bind { var, expr }] => {
            let acc = parse_accessor(inner)?;
            if *expr != Expression::Var(acc.1) || *var == acc.1 || *var == subject_var {
                return None;
            }
            (Some(acc), Some(*var))
        }
        _ => return None,
    };
    let prop_var = accessor.as_ref().map(|(_, v)| *v);
    let select_vars = query.output.projected_vars()?;

    let kind = match &query.grouping {
        // Scalars: single implicit group, aggregates only.
        Some(Grouping::Implicit {
            aggregation: Aggregation { aggregates, binds },
            having: None,
        }) => {
            if !binds.is_empty() || select_vars.len() != aggregates.len() {
                return None;
            }
            let mut tasks = Vec::with_capacity(select_vars.len());
            for out in &select_vars {
                let spec = aggregates.iter().find(|a| a.output_var == *out)?;
                tasks.push(classify_aggregate(&spec.function, subject_var, prop_var)?);
            }
            FoldKind::Scalars(tasks)
        }
        // Histogram: GROUP BY the accessor value, one COUNT(*) / COUNT(?n).
        Some(Grouping::Explicit {
            group_by,
            aggregation: Some(Aggregation { aggregates, binds }),
            having: None,
        }) => {
            let prop_var = prop_var?;
            // The group key is the accessor var or its projection alias
            // (identity bind — identical value per row).
            let key = *group_by.first();
            if group_by.len() != 1 || (key != prop_var && Some(key) != prop_alias) {
                return None;
            }
            if aggregates.len() != 1 || !binds.is_empty() {
                return None;
            }
            let agg = aggregates.first();
            match &agg.function {
                AggregateFn::CountAll => {}
                AggregateFn::Count(v) if *v == subject_var => {}
                _ => return None,
            }
            if select_vars.len() != 2 {
                return None;
            }
            let prop_col = select_vars.iter().position(|v| *v == key)?;
            let count_col = select_vars.iter().position(|v| *v == agg.output_var)?;
            if prop_col == count_col {
                return None;
            }
            FoldKind::Histogram {
                prop_col,
                count_col,
            }
        }
        _ => return None,
    };

    Some(WholeGraphAggPlan {
        anchor,
        accessor: accessor.map(|(p, _)| p),
        kind,
        schema: select_vars,
    })
}

/// Match `Subquery(SELECT DISTINCT ?n WHERE { ?n ?p ?o })` (no modifiers) and
/// return `?n`. The body's `?p`/`?o` never escape (only `?n` is selected), so
/// correlation cannot arise regardless of the `uncorrelated` flag.
fn distinct_subject_scan_var(sq: &crate::ir::SubqueryPattern) -> Option<VarId> {
    if !sq.distinct
        || sq.limit.is_some()
        || sq.offset.is_some()
        || !sq.ordering.is_empty()
        || !sq.order_binds.is_empty()
        || sq.grouping.is_some()
    {
        return None;
    }
    let [subject_var] = sq.select.as_slice() else {
        return None;
    };
    let [Pattern::Triple(tp)] = sq.patterns.as_slice() else {
        return None;
    };
    if tp.dtc.is_some() {
        return None;
    }
    let (Ref::Var(sv), Ref::Var(pv), Term::Var(ov)) = (&tp.s, &tp.p, &tp.o) else {
        return None;
    };
    if sv != subject_var || pv == sv || ov == sv || pv == ov {
        return None;
    }
    Some(*subject_var)
}

fn classify_aggregate(
    function: &AggregateFn,
    subject_var: VarId,
    prop_var: Option<VarId>,
) -> Option<AggTask> {
    let is_prop = |v: VarId| prop_var == Some(v);
    match function {
        AggregateFn::CountAll => Some(AggTask::CountRows),
        AggregateFn::Count(v) if *v == subject_var => Some(AggTask::CountRows),
        AggregateFn::Count(v) if is_prop(*v) => Some(AggTask::CountProp),
        AggregateFn::CountDistinct(v) if *v == subject_var => Some(AggTask::CountSubjects),
        AggregateFn::CountDistinct(v) if is_prop(*v) => Some(AggTask::CountDistinctProp),
        AggregateFn::Min(v) if is_prop(*v) => Some(AggTask::MinProp),
        AggregateFn::Max(v) if is_prop(*v) => Some(AggTask::MaxProp),
        AggregateFn::Avg(v, InputSemantics::List) if is_prop(*v) => Some(AggTask::AvgProp),
        AggregateFn::Sum(v, InputSemantics::List) if is_prop(*v) => Some(AggTask::SumProp),
        _ => None,
    }
}

/// Histograms above this many distinct property values decline to the
/// general pipeline (bounds the single output batch).
const HISTOGRAM_MAX_GROUPS: usize = 65_536;

/// Create the fused operator; declines to `fallback` whenever any component
/// cannot be answered exactly.
pub(crate) fn whole_graph_scalar_aggs_operator(
    plan: WholeGraphAggPlan,
    fallback: Option<BoxedOperator>,
) -> FastPathOperator {
    let schema: Arc<[VarId]> = Arc::from(plan.schema.clone().into_boxed_slice());
    FastPathOperator::with_schema(
        schema.clone(),
        move |ctx| {
            let Some(store) = fast_path_store(ctx) else {
                return Ok(None);
            };

            // Subject universe.
            let subjects = match &plan.anchor {
                Anchor::AllSubjects => {
                    if graph_has_scan_hidden_predicates(ctx, store)? {
                        return Ok(None);
                    }
                    // Distinct subjects from SPOT leaflet lead groups
                    // (SPOT key layout: s_id(8) + …).
                    count_distinct_lead_groups(store, ctx.binary_g_id, RunSortOrder::Spot, 8)?
                }
                Anchor::Class(class_sid) => {
                    let Some(class) = class_stats(ctx, class_sid) else {
                        return Ok(None);
                    };
                    class.count
                }
            };
            if subjects == 0 {
                // Empty universe: leave empty-input aggregate identities
                // (COUNT 0 / unbound MIN) to the general pipeline.
                return Ok(None);
            }

            let accessor = match &plan.accessor {
                Some(pred) => {
                    let pred_sid = normalize_pred_sid(store, pred)?;
                    let Some(p_id) = store.sid_to_p_id(&pred_sid) else {
                        // Absent predicate: all-null accessor rows; the
                        // fallback computes the null-heavy identities.
                        return Ok(None);
                    };
                    let rows = count_rows_for_predicate_psot(store, ctx.binary_g_id, p_id)?;
                    // Class anchor: property-derived folds are predicate-
                    // scoped, so every P-bearing subject must be an instance
                    // of the class. Equality of the per-(class, property)
                    // flake count with the predicate's total row count proves
                    // exactly that (both current-state-exact at HEAD).
                    if let Anchor::Class(class_sid) = &plan.anchor {
                        let Some(class) = class_stats(ctx, class_sid) else {
                            return Ok(None);
                        };
                        let class_prop_rows: u64 = class
                            .properties
                            .iter()
                            .find(|p| p.property_sid == pred_sid)
                            .map(|p| p.datatypes.iter().map(|&(_, c)| c).sum())
                            .unwrap_or(0);
                        if class_prop_rows != rows {
                            return Ok(None);
                        }
                    }
                    let Some(with_prop) =
                        count_distinct_subjects_for_predicate(store, ctx.binary_g_id, p_id)?
                    else {
                        return Ok(None);
                    };
                    Some(AccessorCounts {
                        pred: pred.clone(),
                        p_id,
                        rows,
                        subjects_with: with_prop,
                    })
                }
                None => None,
            };

            let batch = match &plan.kind {
                FoldKind::Scalars(tasks) => {
                    let mut row = Vec::with_capacity(tasks.len());
                    for task in tasks {
                        let Some(binding) = compute_task(
                            *task,
                            store,
                            ctx.binary_g_id,
                            subjects,
                            accessor.as_ref(),
                        )?
                        else {
                            return Ok(None);
                        };
                        row.push(binding);
                    }
                    Batch::single_row(schema.clone(), row)
                        .map_err(|e| QueryError::execution(format!("whole-graph agg batch: {e}")))?
                }
                FoldKind::Histogram {
                    prop_col,
                    count_col,
                } => {
                    let Some(a) = accessor.as_ref() else {
                        return Ok(None);
                    };
                    let Some(b) =
                        compute_histogram(ctx, store, a, subjects, &schema, *prop_col, *count_col)?
                    else {
                        return Ok(None);
                    };
                    b
                }
            };
            Ok(Some(batch))
        },
        fallback,
        "whole-graph scalar aggregates",
    )
}

/// The per-graph class stats entry for `class_sid`, if present.
fn class_stats<'a>(
    ctx: &'a crate::context::ExecutionContext<'_>,
    class_sid: &Sid,
) -> Option<&'a fluree_db_core::index_stats::ClassStatEntry> {
    ctx.active_snapshot
        .stats
        .as_ref()?
        .graphs
        .as_ref()?
        .iter()
        .find(|g| g.g_id == ctx.binary_g_id)?
        .classes
        .as_ref()?
        .iter()
        .find(|c| &c.class_sid == class_sid)
}

/// `GROUP BY ?prop COUNT(*)`: per-value counts from a POST run-length pass
/// (values are contiguous in POST order) plus one null-group row for
/// subjects without the property. Declines on very wide histograms and on
/// stores the group-count walk cannot serve.
fn compute_histogram(
    ctx: &crate::context::ExecutionContext<'_>,
    store: &Arc<BinaryIndexStore>,
    accessor: &AccessorCounts,
    subjects: u64,
    schema: &Arc<[VarId]>,
    prop_col: usize,
    count_col: usize,
) -> Result<Option<Batch>> {
    // Bound the output (one batch) and guarantee the top-K walk keeps every
    // group: the directory-level distinct-object count must fit the cap.
    match count_distinct_objects_for_predicate(store, ctx.binary_g_id, accessor.p_id)? {
        Some(distinct) if (distinct as usize) <= HISTOGRAM_MAX_GROUPS => {}
        _ => return Ok(None),
    }
    let Ok(groups) = crate::fast_group_count_firsts::group_count_v6(
        store,
        ctx.binary_g_id,
        &accessor.pred,
        HISTOGRAM_MAX_GROUPS,
        &ctx.cancellation,
    ) else {
        return Ok(None);
    };

    let view = fluree_db_binary_index::BinaryGraphView::with_novelty(
        Arc::clone(store),
        ctx.binary_g_id,
        ctx.dict_novelty.clone(),
    )
    .with_namespace_codes_fallback(ctx.namespace_codes_fallback.clone());

    let null_rows = subjects.saturating_sub(accessor.subjects_with);
    let mut col_prop: Vec<Binding> = Vec::with_capacity(groups.len() + 1);
    let mut col_count: Vec<Binding> = Vec::with_capacity(groups.len() + 1);
    for (o_type, o_key, count) in groups {
        if o_type == fluree_db_core::o_type::OType::IRI_REF.as_u16() {
            col_prop.push(Binding::encoded_sid(o_key));
        } else {
            let val = view
                .decode_value(o_type, o_key, accessor.p_id)
                .map_err(|e| QueryError::Internal(format!("histogram decode: {e}")))?;
            let dt = store
                .resolve_datatype_sid_for_value(o_type, &val)
                .unwrap_or_else(|| Sid::new(0, ""));
            let dtc = match store.resolve_lang_tag(o_type) {
                Some(tag) => fluree_db_core::DatatypeConstraint::LangTag(Arc::from(tag)),
                None => fluree_db_core::DatatypeConstraint::Explicit(dt),
            };
            col_prop.push(Binding::Lit {
                val,
                dtc,
                t: None,
                op: None,
                p_id: None,
            });
        }
        col_count.push(Binding::lit(FlakeValue::Long(count), Sid::xsd_integer()));
    }
    if null_rows > 0 {
        col_prop.push(Binding::Unbound);
        col_count.push(Binding::lit(
            FlakeValue::Long(count_to_i64(null_rows, "histogram null group")?),
            Sid::xsd_integer(),
        ));
    }

    let mut cols: Vec<Vec<Binding>> = vec![Vec::new(), Vec::new()];
    cols[prop_col] = col_prop;
    cols[count_col] = col_count;
    let batch = Batch::new(schema.clone(), cols)
        .map_err(|e| QueryError::execution(format!("histogram batch: {e}")))?;
    Ok(Some(batch))
}

struct AccessorCounts {
    pred: Ref,
    p_id: u32,
    rows: u64,
    subjects_with: u64,
}

fn compute_task(
    task: AggTask,
    store: &Arc<BinaryIndexStore>,
    g_id: GraphId,
    subjects: u64,
    accessor: Option<&AccessorCounts>,
) -> Result<Option<Binding>> {
    if task.needs_accessor() && accessor.is_none() {
        return Ok(None);
    }
    let int = |v: u64| -> Result<Option<Binding>> {
        Ok(Some(Binding::lit(
            FlakeValue::Long(count_to_i64(v, "whole-graph aggregate")?),
            Sid::xsd_integer(),
        )))
    };
    match task {
        AggTask::CountRows => match accessor {
            // Left join: subjects without P keep 1 row, subjects with P get
            // one row per value.
            Some(a) => int(subjects
                .saturating_sub(a.subjects_with)
                .saturating_add(a.rows)),
            None => int(subjects),
        },
        AggTask::CountSubjects => int(subjects),
        AggTask::CountProp => int(accessor.unwrap().rows),
        AggTask::CountDistinctProp => {
            match count_distinct_objects_for_predicate(store, g_id, accessor.unwrap().p_id)? {
                Some(distinct) => int(distinct),
                None => Ok(None),
            }
        }
        AggTask::MinProp | AggTask::MaxProp => {
            let mode = if task == AggTask::MinProp {
                MinMaxMode::Min
            } else {
                MinMaxMode::Max
            };
            let p_id = accessor.unwrap().p_id;
            if let Some(b) = minmax_numeric_post(store, g_id, p_id, mode)? {
                return Ok(Some(b));
            }
            minmax_string_dict_post(store, g_id, p_id, mode)
        }
        AggTask::AvgProp | AggTask::SumProp => {
            let kind = if task == AggTask::AvgProp {
                ScalarAggKind::AvgNumeric
            } else {
                ScalarAggKind::Sum(SumExprI64::Identity)
            };
            match scan_predicate_scalar_agg(store, g_id, &accessor.unwrap().pred, kind)? {
                Some(output) => Ok(Some(output.into_binding())),
                None => Ok(None),
            }
        }
    }
}

/// Whether the graph carries any predicate the variable-predicate scan hides —
/// `f:reifies*` in every graph, the broader `f:` namespace in the default
/// graph (mirrors `BinaryScanOperator::is_internal_predicate`). Such facts are
/// invisible to the `?n ?p ?o` pipeline but present in the SPOT directories
/// this fold reads, so their presence makes the fold inexact. Errs toward
/// declining when the per-graph stats are unavailable.
fn graph_has_scan_hidden_predicates(
    ctx: &crate::context::ExecutionContext<'_>,
    store: &BinaryIndexStore,
) -> Result<bool> {
    let Some(graphs) = ctx
        .active_snapshot
        .stats
        .as_ref()
        .and_then(|s| s.graphs.as_ref())
    else {
        return Ok(true);
    };
    let Some(g) = graphs.iter().find(|g| g.g_id == ctx.binary_g_id) else {
        return Ok(true);
    };
    for prop in &g.properties {
        if prop.count == 0 {
            continue;
        }
        let Some(sid) = store.predicate_sid(prop.p_id) else {
            return Ok(true);
        };
        if fluree_db_core::is_reserved_reifies_predicate(&sid)
            || (ctx.binary_g_id == 0 && sid.namespace_code == fluree_vocab::namespaces::FLUREE_DB)
        {
            return Ok(true);
        }
    }
    Ok(false)
}
