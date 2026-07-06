//! Conditional Cypher writes.
//!
//! Most Cypher writes lower to a single declarative `Txn` (one WHERE plus
//! delete/insert templates). A few need to **branch or error based on the
//! latest pre-write state** — they can't be represented faithfully by one
//! `Txn`:
//!
//! - `MERGE … ON MATCH SET` — create-and-`ON CREATE` if absent, else
//!   `ON MATCH SET` (mutually exclusive guards).
//! - bare `DELETE n` — must error if `n` still has relationships.
//! - `DELETE r` — must know whether retracting the base edge would affect
//!   parallel relationship identities.
//!
//! These resolve as a [`WritePlan::Conditional`]: the executor probes the
//! current writer snapshot, then resolves to a concrete `Txn` (or a typed
//! Cypher error) which commits through the **same** staging path as any other
//! write — keeping identity, policy, tracking, provenance, index config, cache
//! freshness, and retry behavior consistent. This mirrors how UPSERT reads
//! current state, derives the actual write, and commits it as one transaction.

use fluree_db_cypher::ast::{
    BinOp, CypherAst, DeleteClause, Direction, Expr, FuncCall, Literal, MatchClause, MergeClause,
    NodePattern, Pattern, PatternPart, ProjectionItem, Query, ReadClause, ReturnClause, SetClause,
    Statement, Update, Variable, WithClause, WriteClause,
};
use fluree_db_ledger::LedgerState;
use fluree_db_transact::ir::Txn;

/// A lowered Cypher write: either a ready-to-stage `Txn`, or a conditional
/// write that must probe the writer snapshot before it can be resolved.
pub enum WritePlan {
    /// A ready-to-stage transaction.
    Single(Box<Txn>),
    /// A write needing a pre-write probe. Boxed (both variants embed a large
    /// AST clause).
    Conditional(Box<ConditionalCypherWrite>),
}

/// A write that needs a pre-write probe to choose between branches.
pub enum ConditionalCypherWrite {
    /// Single-node `MERGE` with a non-empty `ON MATCH SET`: probe existence,
    /// then stage the create branch (when absent) or the `ON MATCH SET`
    /// (when present).
    MergeOnMatch(MergeClause),
    /// Bare `MATCH … DELETE n` (non-`DETACH`): probe whether any matched node
    /// still has a relationship, error if so, otherwise stage the node
    /// retraction (via the `DETACH DELETE` lowering — equivalent when there
    /// are no relationships).
    DeleteNode(Update),
    /// `MATCH (a)-[r:T]->(b) DELETE r`: probe whether the matched edge has
    /// parallel siblings (a shared `(s,p,o)` carrying multiple annotation
    /// SIDs), reject if so, otherwise stage the base-edge retraction (the
    /// `f:reifies*` cascade removes the bundle).
    DeleteRel(Update),
}

/// Detect a write shape that requires a pre-write probe. Returns `None` for
/// the common single-`Txn` shapes (which lower directly).
pub fn detect_conditional(ast: &CypherAst) -> Option<ConditionalCypherWrite> {
    let Statement::Update(u) = &ast.statement else {
        return None;
    };
    if u.write_clauses.len() != 1 {
        return None;
    }
    match &u.write_clauses[0] {
        // MERGE … ON MATCH SET: standalone single-node MERGE with on-match.
        WriteClause::Merge(m) => {
            let single_node = m.pattern.parts.len() == 1 && m.pattern.parts[0].tail.is_empty();
            if u.read_clauses.is_empty() && !m.on_match.is_empty() && single_node {
                Some(ConditionalCypherWrite::MergeOnMatch(m.clone()))
            } else {
                None
            }
        }
        // DELETE needs a MATCH. All-relationship-variable targets → DeleteRel
        // (parallel-edge probe). All-node-variable bare targets → DeleteNode
        // (relationship-existence probe). DETACH DELETE (node) and mixed/other
        // shapes lower directly (the lowering handles or rejects them).
        WriteClause::Delete(d) => {
            if u.read_clauses.is_empty() {
                return None;
            }
            // A `WITH` between the MATCH and the DELETE re-scopes/renames
            // variables (`WITH a AS p`, or dropping a rel var). The DELETE
            // classifier and the rel-var lowering key off the *raw* MATCH
            // variables, so they can't honor that horizon — route WITH+DELETE to
            // the single-Txn lowering, which rejects it with a clear error rather
            // than mis-classifying or deleting an out-of-scope variable.
            if u.read_clauses
                .iter()
                .any(|c| matches!(c, ReadClause::With(_)))
            {
                return None;
            }
            let rel_targets = d.targets.iter().filter(|t| is_rel_var(u, &t.name)).count();
            if rel_targets == d.targets.len() && rel_targets > 0 {
                Some(ConditionalCypherWrite::DeleteRel(u.clone()))
            } else if rel_targets == 0 && !d.detach {
                Some(ConditionalCypherWrite::DeleteNode(u.clone()))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// True if `name` is bound as a *relationship* variable in any MATCH.
fn is_rel_var(u: &Update, name: &str) -> bool {
    u.read_clauses.iter().any(|clause| {
        let (ReadClause::Match(m) | ReadClause::OptionalMatch(m)) = clause else {
            return false;
        };
        m.pattern
            .parts
            .iter()
            .flat_map(|p| &p.tail)
            .any(|(rel, _)| rel.var.as_ref().is_some_and(|v| v.name == name))
    })
}

/// True if `name` is bound as a node variable by a **mandatory** (non-OPTIONAL)
/// MATCH. Used to reject bare DELETE targets that are only optionally bound
/// (the relationship probe could otherwise bind an unrelated relationship).
pub(crate) fn bound_by_mandatory_match(u: &Update, name: &str) -> bool {
    u.read_clauses.iter().any(|clause| {
        let ReadClause::Match(m) = clause else {
            return false;
        };
        m.pattern.parts.iter().any(|part| {
            let mut nodes = std::iter::once(&part.head).chain(part.tail.iter().map(|(_, n)| n));
            nodes.any(|n| n.var.as_ref().is_some_and(|v| v.name == name))
        })
    })
}

/// Find the (subject-side, object-side) endpoint variables of relationship
/// variable `rel_var` in the MATCH, honoring direction. Returns `None` if the
/// endpoints aren't both named.
pub(crate) fn rel_endpoint_vars(u: &Update, rel_var: &str) -> Option<(Variable, Variable)> {
    for clause in &u.read_clauses {
        let (ReadClause::Match(m) | ReadClause::OptionalMatch(m)) = clause else {
            continue;
        };
        for part in &m.pattern.parts {
            let mut prev = &part.head;
            for (rel, next) in &part.tail {
                if rel.var.as_ref().is_some_and(|v| v.name == rel_var) {
                    let (s, o) = match rel.direction {
                        Direction::Incoming => (next, prev),
                        _ => (prev, next),
                    };
                    return Some((s.var.clone()?, o.var.clone()?));
                }
                prev = next;
            }
        }
    }
    None
}

/// Build a probe that returns at most one row when a node matching the MERGE
/// identity pattern exists: `MATCH (<node>) RETURN <var> LIMIT 1`. Built from
/// the AST (the node is cloned verbatim) so labels/properties — including
/// backtick-quoted identifiers — round-trip exactly, with no text
/// serialization. The node carries a variable (required for ON MATCH SET).
pub(crate) fn build_merge_probe_ast(node: &NodePattern) -> CypherAst {
    let span = node.span;
    let var = node
        .var
        .clone()
        .expect("MERGE node has a variable (checked before resolution)");
    CypherAst {
        statement: Statement::Query(Query {
            clauses: vec![ReadClause::Match(MatchClause {
                pattern: Pattern {
                    parts: vec![PatternPart {
                        path_var: None,
                        path_search: None,
                        head: node.clone(),
                        tail: Vec::new(),
                        span,
                    }],
                    span,
                },
                where_clause: None,
                span,
            })],
            return_clause: ReturnClause {
                items: vec![ProjectionItem {
                    expr: Expr::Var(var),
                    alias: None,
                    span,
                }],
                distinct: false,
                order_by: Vec::new(),
                skip: None,
                limit: Some(Expr::Lit(Literal::Integer(1, span))),
                span,
            },
            union_tail: None,
            span,
        }),
        span,
    }
}

/// Build the on-match branch: `MATCH (pattern) SET <on_match>`. Reuses the
/// existing MATCH … SET lowering.
pub(crate) fn build_on_match_ast(merge: &MergeClause) -> CypherAst {
    let span = merge.span;
    CypherAst {
        statement: Statement::Update(Update {
            read_clauses: vec![ReadClause::Match(MatchClause {
                pattern: merge.pattern.clone(),
                where_clause: None,
                span,
            })],
            write_clauses: vec![WriteClause::Set(SetClause {
                items: merge.on_match.clone(),
                span,
            })],
            return_clause: None,
            span,
        }),
        span,
    }
}

/// The DELETE clause inside a `DeleteNode` plan (its single write clause).
pub(crate) fn delete_clause(update: &Update) -> Option<&DeleteClause> {
    match update.write_clauses.first() {
        Some(WriteClause::Delete(d)) => Some(d),
        _ => None,
    }
}

/// Build a probe over the original MATCH clauses that returns candidate nodes
/// for a bare `DELETE n` target. The API conditional-write path appends its
/// own internal triple pattern after lowering so it can inspect predicate/object
/// bindings without going through Cypher relationship-variable sidecar matching.
pub(crate) fn build_delete_target_probe_ast(
    read_clauses: &[ReadClause],
    target: &Variable,
) -> CypherAst {
    let span = target.span;
    CypherAst {
        statement: Statement::Query(Query {
            clauses: read_clauses.to_vec(),
            return_clause: ReturnClause {
                items: vec![ProjectionItem {
                    expr: Expr::Var(target.clone()),
                    alias: None,
                    span,
                }],
                distinct: false,
                order_by: Vec::new(),
                skip: None,
                limit: None,
                span,
            },
            union_tail: None,
            span,
        }),
        span,
    }
}

/// Build a probe that returns at most one row when the matched relationship
/// `rel_var` has a **parallel sibling** — another reified edge sharing the same
/// `(a)-[:T]->(b)` base triple. Appends
/// `WITH <a>, <b>, count(<rel_var>) AS __cyrel_c WHERE __cyrel_c > 1
///  RETURN <a> LIMIT 1` to the original read clauses. Named relationships bind
/// one row per annotation SID, so a `count > 1` per `(a, b)` group means the
/// base edge backs multiple relationship identities — retracting it would
/// disturb the siblings, so `DELETE r` must reject.
pub(crate) fn build_parallel_probe_ast(
    read_clauses: &[ReadClause],
    a: &Variable,
    b: &Variable,
    rel_var: &str,
) -> CypherAst {
    let span = a.span;
    let proj = |v: &Variable| ProjectionItem {
        expr: Expr::Var(v.clone()),
        alias: None,
        span,
    };
    let count_alias = Variable {
        name: "__cyrel_c".to_string(),
        span,
    };
    let count_item = ProjectionItem {
        // DISTINCT: count relationship *identities*, not solution rows. Extra
        // multiplicity in the original MATCH (another matched variable) can
        // repeat one identity across rows, which would falsely trip the guard.
        expr: Expr::Call(FuncCall {
            name: "count".to_string(),
            args: vec![Expr::Var(Variable {
                name: rel_var.to_string(),
                span,
            })],
            distinct: true,
            span,
        }),
        alias: Some(count_alias.clone()),
        span,
    };
    let having = Expr::BinOp(
        BinOp::Gt,
        Box::new(Expr::Var(count_alias)),
        Box::new(Expr::Lit(Literal::Integer(1, span))),
        span,
    );
    let mut clauses: Vec<ReadClause> = read_clauses.to_vec();
    clauses.push(ReadClause::With(WithClause {
        items: vec![proj(a), proj(b), count_item],
        distinct: false,
        where_clause: Some(having),
        order_by: Vec::new(),
        skip: None,
        limit: None,
        span,
    }));

    CypherAst {
        statement: Statement::Query(Query {
            clauses,
            return_clause: ReturnClause {
                items: vec![proj(a)],
                distinct: false,
                order_by: Vec::new(),
                skip: None,
                limit: Some(Expr::Lit(Literal::Integer(1, span))),
                span,
            },
            union_tail: None,
            span,
        }),
        span,
    }
}

/// Build the deletion branch for a verified-relationship-free `DELETE n`: the
/// same statement as `DETACH DELETE n` (equivalent when there are no
/// relationships), which lowers to the in/out-bound retraction templates.
pub(crate) fn build_detach_delete_ast(update: &Update) -> CypherAst {
    let mut u = update.clone();
    for w in &mut u.write_clauses {
        if let WriteClause::Delete(d) = w {
            d.detach = true;
        }
    }
    CypherAst {
        statement: Statement::Update(u),
        span: update.span,
    }
}

/// Build the create branch: the MERGE with `ON MATCH SET` cleared (so it
/// lowers to the single-Txn create-if-absent path with `ON CREATE SET`).
pub(crate) fn build_create_ast(merge: &MergeClause) -> CypherAst {
    let span = merge.span;
    CypherAst {
        statement: Statement::Update(Update {
            read_clauses: Vec::new(),
            write_clauses: vec![WriteClause::Merge(MergeClause {
                pattern: merge.pattern.clone(),
                on_create: merge.on_create.clone(),
                on_match: Vec::new(),
                span,
            })],
            return_clause: None,
            span,
        }),
        span,
    }
}

// ---- Write-statement RETURN (created entities) ------------------------------

/// One column of a validated write-statement `RETURN`: the display name and
/// the blank-node label (sans `_:`) of the created entity the variable names.
#[derive(Debug, Clone)]
pub struct CypherReturnColumn {
    pub name: String,
    /// The `{label}` component of the skolem key `fdb-{txn_id}-{solution}-{label}`.
    pub label: String,
}

/// A validated plan for answering `CREATE … RETURN …` after the commit. The
/// created entities' Sids are fully determined by the transaction's skolem id
/// (supplied via `TxnOpts::skolem_txn_id`) plus the WHERE solution index, so
/// the rows are reconstructed post-commit without threading state out of
/// staging.
#[derive(Debug, Clone)]
pub struct CypherWriteReturnPlan {
    pub columns: Vec<CypherReturnColumn>,
    /// With no read clauses the templates fire exactly once (one solution);
    /// otherwise the solution count is discovered by existence probes.
    pub has_read_clauses: bool,
}

/// Upper bound on WHERE solutions a write RETURN will reconstruct. Exceeding
/// it errors rather than silently truncating.
const MAX_WRITE_RETURN_SOLUTIONS: u64 = 4096;

/// Validate and plan a trailing `RETURN` on a Cypher write statement.
/// `Ok(None)` when the statement is a read or has no RETURN. v1 surface: bare
/// variables (optionally aliased) naming entities *created* by this
/// statement — a fresh CREATE node variable or a CREATE relationship
/// variable. Everything else gets a clear deferred error.
pub fn plan_write_return(ast: &CypherAst) -> Result<Option<CypherWriteReturnPlan>, String> {
    let Statement::Update(u) = &ast.statement else {
        return Ok(None);
    };
    let Some(rc) = &u.return_clause else {
        return Ok(None);
    };
    if rc.distinct || !rc.order_by.is_empty() || rc.skip.is_some() || rc.limit.is_some() {
        return Err(
            "DISTINCT / ORDER BY / SKIP / LIMIT on a write-statement RETURN are deferred"
                .to_string(),
        );
    }
    if u.write_clauses
        .iter()
        .any(|w| matches!(w, WriteClause::Merge(_)))
    {
        return Err(
            "RETURN with MERGE is deferred — the matched branch's node is not a created \
             entity, so it can't be reconstructed post-commit"
                .to_string(),
        );
    }

    // Variables bound by the read side (MATCH / WITH / UNWIND / InlineRows) —
    // these reference existing data, not created entities.
    let mut read_bound: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for c in &u.read_clauses {
        match c {
            ReadClause::Match(m) | ReadClause::OptionalMatch(m) => {
                for part in &m.pattern.parts {
                    collect_part_vars(part, &mut read_bound);
                }
            }
            ReadClause::With(w) => {
                for item in &w.items {
                    if let Some(a) = &item.alias {
                        read_bound.insert(a.name.as_str());
                    } else if let Expr::Var(v) = &item.expr {
                        read_bound.insert(v.name.as_str());
                    }
                }
            }
            ReadClause::Unwind(uw) => {
                read_bound.insert(uw.alias.name.as_str());
            }
            ReadClause::InlineRows { vars, .. } => {
                for v in vars {
                    read_bound.insert(v.name.as_str());
                }
            }
            ReadClause::CallSubquery(_) => {}
        }
    }

    // Entities created by CREATE clauses.
    let mut created_nodes: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut created_rels: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for w in &u.write_clauses {
        let WriteClause::Create(c) = w else { continue };
        for part in &c.pattern.parts {
            for node in std::iter::once(&part.head).chain(part.tail.iter().map(|(_, n)| n)) {
                if let Some(v) = &node.var {
                    if !read_bound.contains(v.name.as_str()) {
                        created_nodes.insert(v.name.as_str());
                    }
                }
            }
            for (rel, _) in &part.tail {
                if let Some(v) = &rel.var {
                    created_rels.insert(v.name.as_str());
                }
            }
        }
    }

    let mut columns = Vec::with_capacity(rc.items.len());
    for item in &rc.items {
        let Expr::Var(v) = &item.expr else {
            return Err(
                "a write-statement RETURN supports bare created-entity variables in v1 \
                 (expressions are deferred)"
                    .to_string(),
            );
        };
        let name = item
            .alias
            .as_ref()
            .map_or_else(|| v.name.clone(), |a| a.name.clone());
        let label = if created_rels.contains(v.name.as_str()) {
            format!("cy_rel_{}", v.name)
        } else if created_nodes.contains(v.name.as_str()) {
            format!("cy_{}", v.name)
        } else {
            return Err(format!(
                "RETURN of `{}` on a write statement is deferred — only entities created \
                 by this statement (a fresh CREATE node or relationship variable) can be \
                 returned in v1",
                v.name
            ));
        };
        columns.push(CypherReturnColumn { name, label });
    }

    Ok(Some(CypherWriteReturnPlan {
        columns,
        has_read_clauses: !u.read_clauses.is_empty(),
    }))
}

fn collect_part_vars<'a>(part: &'a PatternPart, out: &mut std::collections::HashSet<&'a str>) {
    for node in std::iter::once(&part.head).chain(part.tail.iter().map(|(_, n)| n)) {
        if let Some(v) = &node.var {
            out.insert(v.name.as_str());
        }
    }
    for (rel, _) in &part.tail {
        if let Some(v) = &rel.var {
            out.insert(v.name.as_str());
        }
    }
}

/// The skolemized Sid a created entity resolves to (mirrors
/// `FlakeGenerator::skolemize_blank_node`: key `{txn_id}-{solution}-{label}`,
/// blank-node local `fdb-{key}`).
fn skolem_sid(skolem_txn_id: &str, solution: u64, label: &str) -> fluree_db_core::Sid {
    let local = format!(
        "{}-{skolem_txn_id}-{solution}-{label}",
        fluree_db_transact::BLANK_NODE_ID_PREFIX
    );
    fluree_db_core::Sid::new(fluree_vocab::namespaces::BLANK_NODE, local)
}

/// Reconstruct the rows for a write-statement RETURN against the post-commit
/// ledger state, as a Cypher-JSON envelope
/// (`{"results":[{"columns":[…],"data":[{"row":[…],"meta":[…]}]}]}`).
///
/// Solutions are contiguous indices `0..n`; with read clauses present, `n` is
/// discovered by probing the first column's skolem Sid per solution (a
/// one-flake SPOT range). Entities serialize as their blank-node identifier
/// string (`_:fdb-…`), matching the minimal node serialization of the read
/// path.
pub async fn write_return_rows(
    plan: &CypherWriteReturnPlan,
    skolem_txn_id: &str,
    ledger: &LedgerState,
) -> Result<serde_json::Value, crate::error::ApiError> {
    use fluree_db_core::{IndexType, RangeMatch, RangeOptions, RangeTest};

    let solutions: u64 = if plan.has_read_clauses {
        let probe_col = &plan.columns[0];
        let overlay: &dyn fluree_db_core::OverlayProvider = ledger.novelty.as_ref();
        let mut n = 0u64;
        loop {
            if n >= MAX_WRITE_RETURN_SOLUTIONS {
                return Err(crate::error::ApiError::cypher(
                    format!(
                        "write RETURN is capped at {MAX_WRITE_RETURN_SOLUTIONS} rows — drop \
                         the RETURN clause for larger batches"
                    ),
                    Vec::new(),
                ));
            }
            let sid = skolem_sid(skolem_txn_id, n, &probe_col.label);
            let db = fluree_db_core::GraphDbRef::new(
                &ledger.snapshot,
                fluree_db_core::DEFAULT_GRAPH_ID,
                overlay,
                ledger.t(),
            );
            let flakes = db
                .range_with_opts(
                    IndexType::Spot,
                    RangeTest::Eq,
                    RangeMatch::subject(sid),
                    RangeOptions::default().with_flake_limit(1),
                )
                .await
                .map_err(|e| {
                    crate::error::ApiError::internal(format!(
                        "write RETURN existence probe failed: {e}"
                    ))
                })?;
            if flakes.is_empty() {
                break;
            }
            n += 1;
        }
        n
    } else {
        // No WHERE: templates fire once against the single empty solution.
        1
    };

    let columns: Vec<&str> = plan.columns.iter().map(|c| c.name.as_str()).collect();
    let mut data = Vec::with_capacity(solutions as usize);
    for s in 0..solutions {
        let row: Vec<serde_json::Value> = plan
            .columns
            .iter()
            .map(|c| {
                serde_json::Value::String(format!(
                    "{}{}-{skolem_txn_id}-{s}-{}",
                    fluree_db_transact::BLANK_NODE_PREFIX,
                    fluree_db_transact::BLANK_NODE_ID_PREFIX,
                    c.label
                ))
            })
            .collect();
        let meta: Vec<serde_json::Value> = plan
            .columns
            .iter()
            .map(|_| serde_json::Value::Null)
            .collect();
        data.push(serde_json::json!({"row": row, "meta": meta}));
    }
    Ok(serde_json::json!({
        "results": [{"columns": columns, "data": data}]
    }))
}

/// Parse + param-substitute Cypher source and plan its write-statement
/// RETURN. Parse and parameter errors return `Ok(None)` — the consensus
/// lowering path reports those with full diagnostics; only RETURN-shape
/// validation errors surface here.
pub fn plan_write_return_source(
    cypher: &str,
    params: Option<&fluree_db_cypher::ParamMap>,
) -> Result<Option<CypherWriteReturnPlan>, crate::error::ApiError> {
    let Ok(ast) = crate::query::helpers::substituted_cypher_ast(cypher, params) else {
        return Ok(None);
    };
    plan_write_return(&ast).map_err(|e| crate::error::ApiError::cypher(e, Vec::new()))
}

/// A fresh unique skolemization id for [`TxnOpts::skolem_txn_id`]
/// (`fluree_db_transact::ir::TxnOpts`).
pub fn fresh_skolem_txn_id() -> String {
    fluree_db_transact::generate_txn_id()
}

/// Whether a Cypher statement is a write (contains updating clauses).
/// Transports that carry reads and writes on one verb — Bolt `RUN` —
/// dispatch on this. Parses through the process-wide AST cache; parse
/// errors surface with full diagnostics.
pub fn cypher_statement_is_write(cypher: &str) -> crate::Result<bool> {
    let ast = crate::query::helpers::parse_cypher_ast_cached(cypher)?;
    Ok(matches!(
        ast.statement,
        fluree_db_cypher::ast::Statement::Update(_)
    ))
}
