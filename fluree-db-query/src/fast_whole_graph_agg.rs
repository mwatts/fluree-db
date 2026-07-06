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
//! The fold requires the strict metadata-lane gate ([`fast_path_store`]:
//! single-ledger, root/no policy, no overlay, at HEAD) and declines when the
//! graph carries predicates the variable-predicate scan hides (`f:reifies*`
//! anywhere, the `f:` namespace in the default graph) — those facts are
//! invisible to `?n ?p ?o` but not to the SPOT directories this fold reads.

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

/// Detected plan: the accessor predicate (if any) and one task per projected
/// output column, in projection order.
pub(crate) struct WholeGraphAggPlan {
    accessor: Option<Ref>,
    tasks: Vec<AggTask>,
    schema: Vec<VarId>,
}

/// Recognize the whole-graph scalar-aggregate shape. See the module docs for
/// the exact pattern and per-aggregate soundness arguments.
pub(crate) fn detect_whole_graph_scalar_aggs(query: &Query) -> Option<WholeGraphAggPlan> {
    // Single implicit group, aggregates only — no HAVING, post-binds,
    // ordering, offset, DISTINCT output, or LIMIT 0.
    let Some(Grouping::Implicit {
        aggregation: Aggregation { aggregates, binds },
        having: None,
    }) = &query.grouping
    else {
        return None;
    };
    if !binds.is_empty()
        || !query.ordering.is_empty()
        || !query.order_binds.is_empty()
        || query.offset.is_some()
        || query.output.is_distinct()
        || query.limit == Some(0)
        || query.post_values.is_some()
    {
        return None;
    }

    // Pattern shape: the DISTINCT-subject subquery, then at most one
    // single-triple property-accessor OPTIONAL.
    let (first, rest) = query.patterns.split_first()?;
    let Pattern::Subquery(sq) = first else {
        return None;
    };
    let subject_var = distinct_subject_scan_var(sq)?;

    let accessor = match rest {
        [] => None,
        [Pattern::Optional(inner)] => {
            let [Pattern::Triple(tp)] = inner.as_slice() else {
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
        }
        _ => return None,
    };
    let prop_var = accessor.as_ref().map(|(_, v)| *v);

    // Every aggregate must fold, and the projection must be exactly the
    // aggregate outputs.
    let select_vars = query.output.projected_vars()?;
    if select_vars.len() != aggregates.len() {
        return None;
    }
    let mut tasks = Vec::with_capacity(select_vars.len());
    for out in &select_vars {
        let spec = aggregates.iter().find(|a| a.output_var == *out)?;
        tasks.push(classify_aggregate(&spec.function, subject_var, prop_var)?);
    }

    Some(WholeGraphAggPlan {
        accessor: accessor.map(|(p, _)| p),
        tasks,
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
            if graph_has_scan_hidden_predicates(ctx, store)? {
                return Ok(None);
            }

            // N: distinct subjects, from SPOT leaflet lead groups
            // (SPOT key layout: s_id(8) + …).
            let subjects =
                count_distinct_lead_groups(store, ctx.binary_g_id, RunSortOrder::Spot, 8)?;
            if subjects == 0 {
                // Empty graph: leave empty-input aggregate identities
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

            let mut row = Vec::with_capacity(plan.tasks.len());
            for task in &plan.tasks {
                let Some(binding) =
                    compute_task(*task, store, ctx.binary_g_id, subjects, accessor.as_ref())?
                else {
                    return Ok(None);
                };
                row.push(binding);
            }
            let batch = Batch::single_row(schema.clone(), row)
                .map_err(|e| QueryError::execution(format!("whole-graph agg batch: {e}")))?;
            Ok(Some(batch))
        },
        fallback,
        "whole-graph scalar aggregates",
    )
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
