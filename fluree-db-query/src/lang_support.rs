//! Pluggable SPARQL support for policy queries and datalog rules
//!
//! `f:query` policy conditions and `f:rule` datalog rules can be written in
//! SPARQL (stored with the `f:sparql` datatype) in addition to the default
//! JSON-LD query form. The SPARQL parser lives in `fluree-db-sparql`, which
//! *depends on* this crate for lowering — so this crate cannot call it
//! directly. Instead, a higher layer (`fluree-db-api`) registers lowering
//! hooks here at startup via [`register_sparql_support`].
//!
//! Consumers ([`crate::policy::QueryPolicyExecutor`] for policies,
//! [`crate::datalog_rules`] for rules) look the hooks up with
//! [`sparql_support`] and fail closed (policy: deny; rule: skip with an
//! error log) when SPARQL support has not been registered.

use crate::ir::Pattern;
use crate::var_registry::VarRegistry;
use fluree_db_core::LedgerSnapshot;
use std::sync::OnceLock;

// Re-exported so the registering layer can construct [`SparqlRuleParts`]
// without depending on fluree-db-reasoner directly.
pub use fluree_db_reasoner::{CompareOp, RuleFilter, RuleTerm, RuleTriplePattern, RuleValue};

/// Lower a SPARQL ASK/SELECT policy query to WHERE patterns.
///
/// Registers special variables (e.g. `$this`, `$identity`) in `vars` as a
/// side effect of lowering. Returns an error string for parse failures or
/// unsupported query forms (CONSTRUCT/DESCRIBE/UPDATE).
pub type SparqlPolicyLowerFn = fn(
    source: &str,
    snapshot: &LedgerSnapshot,
    vars: &mut VarRegistry,
) -> Result<Vec<Pattern>, String>;

/// Datalog rule parts lowered from a SPARQL `CONSTRUCT ... WHERE ...` rule.
#[derive(Debug)]
pub struct SparqlRuleParts {
    /// Body patterns (the WHERE clause)
    pub where_patterns: Vec<RuleTriplePattern>,
    /// Body filters (FILTER expressions, restricted to comparisons)
    pub filters: Vec<RuleFilter>,
    /// Head patterns (the CONSTRUCT template)
    pub insert_patterns: Vec<RuleTriplePattern>,
}

/// Lower a SPARQL rule (CONSTRUCT form) to datalog rule parts.
///
/// Returns an error string for parse failures or constructs the datalog
/// engine cannot execute (OPTIONAL, UNION, property paths, etc.).
pub type SparqlRuleLowerFn =
    fn(source: &str, snapshot: &LedgerSnapshot) -> Result<SparqlRuleParts, String>;

/// SPARQL lowering hooks registered by a higher layer.
pub struct SparqlSupport {
    /// Policy-query lowering (`f:query` with `f:sparql` datatype)
    pub lower_policy_query: SparqlPolicyLowerFn,
    /// Datalog-rule lowering (`f:rule` with `f:sparql` datatype)
    pub lower_rule: SparqlRuleLowerFn,
}

static SPARQL_SUPPORT: OnceLock<SparqlSupport> = OnceLock::new();

/// Register SPARQL lowering hooks. Idempotent — the first registration wins;
/// later calls are ignored.
pub fn register_sparql_support(support: SparqlSupport) {
    let _ = SPARQL_SUPPORT.set(support);
}

/// Get the registered SPARQL support, if any.
pub fn sparql_support() -> Option<&'static SparqlSupport> {
    SPARQL_SUPPORT.get()
}
