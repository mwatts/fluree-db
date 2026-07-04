//! Vector function implementations
//!
//! Implements vector/embedding functions: dotProduct, cosineSimilarity, euclideanDistance.
//!
//! These run once per bound row (flat ranking over every value of a vector
//! property), so the per-row path is kept allocation-free: arguments are
//! resolved to borrowed `&[f64]` slices straight from the row's bindings
//! (or the expression's constant) instead of round-tripping through
//! `ComparableValue`, which would alloc + copy the full vector — including
//! the query vector, which never changes across rows. The math itself goes
//! through the runtime-dispatched SIMD kernels in [`super::vector_math`].

use std::sync::Arc;

use crate::binding::{Binding, RowAccess};
use crate::context::ExecutionContext;
use crate::error::{QueryError, Result};
use crate::ir::Expression;

use fluree_db_core::FlakeValue;

use super::helpers::check_arity;
use super::value::ComparableValue;
use super::vector_math;

pub fn eval_dot_product<R: RowAccess>(
    args: &[Expression],
    row: &R,
    ctx: Option<&ExecutionContext<'_>>,
) -> Result<Option<ComparableValue>> {
    eval_binary_vector_fn(args, row, ctx, "dotProduct", |a, b| {
        Some(vector_math::dot_f64(a, b))
    })
}

pub fn eval_cosine_similarity<R: RowAccess>(
    args: &[Expression],
    row: &R,
    ctx: Option<&ExecutionContext<'_>>,
) -> Result<Option<ComparableValue>> {
    // Zero-magnitude input is mathematically undefined, not a type error →
    // the kernel returns None and the row's score is unbound.
    eval_binary_vector_fn(args, row, ctx, "cosineSimilarity", vector_math::cosine_f64)
}

pub fn eval_euclidean_distance<R: RowAccess>(
    args: &[Expression],
    row: &R,
    ctx: Option<&ExecutionContext<'_>>,
) -> Result<Option<ComparableValue>> {
    eval_binary_vector_fn(args, row, ctx, "euclideanDistance", |a, b| {
        Some(vector_math::l2_f64(a, b))
    })
}

/// A resolved vector argument: borrowed from the row/expression wherever
/// possible, or a shared handle when the value was decoded (EncodedLit) or
/// came from a nested expression.
enum VecArg<'a> {
    Slice(&'a [f64]),
    Shared(Arc<[f64]>),
}

impl VecArg<'_> {
    fn as_slice(&self) -> &[f64] {
        match self {
            VecArg::Slice(s) => s,
            VecArg::Shared(a) => a,
        }
    }
}

/// Resolve one argument to a vector without copying when avoidable.
///
/// Returns `Ok(None)` for unbound variables and non-vector values (the
/// caller maps that to an unbound result, mirroring the other eval fns).
fn resolve_vector_arg<'a, R: RowAccess>(
    expr: &'a Expression,
    row: &'a R,
    ctx: Option<&ExecutionContext<'_>>,
) -> Result<Option<VecArg<'a>>> {
    match expr {
        // Row/VALUES-bound variable: borrow the vector in place.
        Expression::Var(var) => match row.get(*var) {
            Some(Binding::Lit {
                val: FlakeValue::Vector(v),
                ..
            }) => Ok(Some(VecArg::Slice(v))),
            Some(Binding::EncodedLit {
                o_kind,
                o_key,
                p_id,
                dt_id,
                lang_id,
                ..
            }) => {
                let Some(decoded) = ctx
                    .and_then(|c| c.decode_encoded_value(*o_kind, *o_key, *p_id, *dt_id, *lang_id))
                else {
                    return Ok(None);
                };
                match decoded {
                    Ok(FlakeValue::Vector(v)) => Ok(Some(VecArg::Shared(v))),
                    Ok(_) => Ok(None),
                    Err(e) => Err(QueryError::execution(format!(
                        "decode encoded vector (o_kind={o_kind}, o_key={o_key}, p_id={p_id}): {e}"
                    ))),
                }
            }
            _ => Ok(None),
        },
        // Inline constant vector: borrow from the expression itself.
        Expression::Const(FlakeValue::Vector(v)) => Ok(Some(VecArg::Slice(v))),
        // Anything else (nested expression): evaluate and keep the Arc.
        _ => match expr.eval_to_comparable(row, ctx)? {
            Some(ComparableValue::Vector(v)) => Ok(Some(VecArg::Shared(v))),
            _ => Ok(None),
        },
    }
}

/// Evaluate a binary vector function over borrowed slices.
fn eval_binary_vector_fn<R: RowAccess, F>(
    args: &[Expression],
    row: &R,
    ctx: Option<&ExecutionContext<'_>>,
    fn_name: &str,
    compute: F,
) -> Result<Option<ComparableValue>>
where
    F: Fn(&[f64], &[f64]) -> Option<f64>,
{
    check_arity(args, 2, fn_name)?;
    let v1 = resolve_vector_arg(&args[0], row, ctx)?;
    let v2 = resolve_vector_arg(&args[1], row, ctx)?;
    match (v1, v2) {
        (Some(a), Some(b)) => {
            let (a, b) = (a.as_slice(), b.as_slice());
            if a.len() != b.len() {
                Err(QueryError::InvalidFilter(format!(
                    "{} requires vectors of equal length (got {} and {})",
                    fn_name,
                    a.len(),
                    b.len()
                )))
            } else {
                Ok(compute(a, b).map(ComparableValue::Double))
            }
        }
        // Type mismatch or unbound -> return None (SPARQL-style graceful handling)
        _ => Ok(None),
    }
}
