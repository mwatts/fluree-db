//! V4 — GROUP BY / aggregate projection-scope validation.
//!
//! SPARQL 1.1 §11 / §18.2.4 and the `SelectClause` grammar note: when a
//! query groups — an explicit `GROUP BY`, or an aggregate in the
//! projection (implicit single group) — every projected variable must be
//! a **group key** (a bare `GROUP BY ?v`, or the alias of a
//! `GROUP BY (expr AS ?v)`) or appear only **inside an aggregate**; and
//! `SELECT *` is not permitted with `GROUP BY`.
//!
//! Leniencies (deliberate, to avoid over-rejection):
//! - `GROUP BY (?v)` — a bracketed bare variable — counts as the key `?v`.
//! - An alias assigned by an *earlier* item in the same SELECT clause is
//!   usable in later projection expressions (`SELECT (SUM(?x) AS ?s)
//!   (?s + 1 AS ?t)` is legal — the Extend chain binds `?s` first).
//! - `HAVING` / `ORDER BY` expressions are not checked here.

use std::collections::HashSet;

use crate::ast::expr::Expression;
use crate::ast::query::{GroupByClause, GroupCondition, SelectVariable, SelectVariables};
use crate::diag::{DiagCode, Diagnostic, Label};
use crate::span::SourceSpan;

/// Check a SELECT clause's projection against its grouping.
///
/// `clause_span` anchors the `SELECT *`-with-`GROUP BY` error (the star
/// itself has no span of its own).
pub(super) fn check_projection_scope(
    variables: &SelectVariables,
    group_by: Option<&GroupByClause>,
    clause_span: SourceSpan,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let items = match variables {
        SelectVariables::Star => {
            // `SELECT *` cannot contain an aggregate, so only an explicit
            // GROUP BY makes it invalid.
            if let Some(group_by) = group_by {
                diagnostics.push(
                    Diagnostic::error(
                        DiagCode::SelectStarWithGroupBy,
                        "SELECT * is not allowed with GROUP BY",
                        clause_span,
                    )
                    .with_label(Label::new(group_by.span, "grouped here"))
                    .with_help(
                        "Project the group keys and aggregates explicitly, \
                         e.g. SELECT ?key (COUNT(*) AS ?n).",
                    ),
                );
            }
            return;
        }
        SelectVariables::Explicit(items) => items,
    };

    let has_aggregate = items.iter().any(|item| match item {
        SelectVariable::Var(_) => false,
        SelectVariable::Expr { expr, .. } => expr.contains_aggregate(),
    });
    if group_by.is_none() && !has_aggregate {
        return; // Not a grouped query.
    }

    // Allowed bare variables: group keys, plus aliases assigned by earlier
    // items in this SELECT clause.
    let mut allowed: HashSet<&str> = HashSet::new();
    if let Some(group_by) = group_by {
        for condition in &group_by.conditions {
            match condition {
                GroupCondition::Var(v) => {
                    allowed.insert(v.name.as_ref());
                }
                GroupCondition::Expr { expr, alias, .. } => {
                    if let Some(alias) = alias {
                        allowed.insert(alias.name.as_ref());
                    } else if let Expression::Var(v) = expr.unwrap_bracketed() {
                        // `GROUP BY (?v)` — treat as the key `?v`.
                        allowed.insert(v.name.as_ref());
                    }
                    // An unaliased key *expression* contributes no variable:
                    // `GROUP BY (?a + ?b)` does not license projecting `?a`.
                }
            }
        }
    }

    for item in items {
        match item {
            SelectVariable::Var(v) => {
                if !allowed.contains(v.name.as_ref()) {
                    diagnostics.push(ungrouped_error(v.name.as_ref(), v.span, group_by));
                }
            }
            SelectVariable::Expr { expr, alias, .. } => {
                let mut reported: HashSet<&str> = HashSet::new();
                for v in expr.unaggregated_variables() {
                    if !allowed.contains(v.name.as_ref()) && reported.insert(v.name.as_ref()) {
                        diagnostics.push(ungrouped_error(v.name.as_ref(), v.span, group_by));
                    }
                }
                // Later projection expressions may use this alias.
                allowed.insert(alias.name.as_ref());
            }
        }
    }
}

fn ungrouped_error(
    name: &str,
    span: SourceSpan,
    group_by: Option<&GroupByClause>,
) -> Diagnostic {
    let mut diag = Diagnostic::error(
        DiagCode::UngroupedVariableInProjection,
        format!(
            "variable ?{name} is projected but is neither a GROUP BY key \
             nor aggregated"
        ),
        span,
    )
    .with_help(
        "In a grouped query every projected variable must be a GROUP BY \
         key or appear only inside an aggregate such as COUNT()/SUM() \
         (SPARQL 1.1 §11).",
    );
    if let Some(group_by) = group_by {
        diag = diag.with_label(Label::new(group_by.span, "grouped here"));
    } else {
        diag = diag.with_note(
            "An aggregate in the projection groups the whole solution into \
             a single implicit group.",
        );
    }
    diag
}
