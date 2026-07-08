//! Policy query executor implementation
//!
//! Implements `PolicyQueryExecutor` using the query engine asynchronously.

use crate::binding::Binding;
use crate::context::ExecutionContext;
use crate::execute::build_where_operators_seeded;
use crate::ir::Pattern;
use crate::var_registry::VarRegistry;
use fluree_db_core::{GraphId, LedgerSnapshot, OverlayProvider, Sid};
use fluree_db_policy::{
    PolicyQuery, PolicyQueryExecutor, PolicyQueryFut, PolicyQueryLanguage, Result as PolicyResult,
    UNBOUND_IDENTITY_PREFIX,
};
use std::collections::HashMap;

/// Policy query executor that runs queries against a database
///
/// This executor converts `PolicyQuery` to the query engine's IR and
/// executes with a root context (no policy filtering).
pub struct QueryPolicyExecutor<'a> {
    /// The database snapshot to query
    pub snapshot: &'a LedgerSnapshot,
    /// Optional overlay provider (for staged flakes)
    pub overlay: Option<&'a dyn OverlayProvider>,
    /// Target transaction time
    pub to_t: i64,
    /// Graph ID for range queries (default: 0 = default graph)
    pub g_id: GraphId,
}

impl<'a> QueryPolicyExecutor<'a> {
    /// Create a new query executor for the default graph
    pub fn new(snapshot: &'a LedgerSnapshot) -> Self {
        Self {
            snapshot,
            overlay: None,
            to_t: snapshot.t,
            g_id: 0,
        }
    }

    /// Create a query executor with overlay support for the default graph
    pub fn with_overlay(
        snapshot: &'a LedgerSnapshot,
        overlay: &'a dyn OverlayProvider,
        to_t: i64,
    ) -> Self {
        Self {
            snapshot,
            overlay: Some(overlay),
            to_t,
            g_id: 0,
        }
    }

    /// Set the graph ID for range queries.
    ///
    /// Policy queries will execute against this graph instead of the default graph.
    pub fn with_graph_id(mut self, g_id: GraphId) -> Self {
        self.g_id = g_id;
        self
    }
}

impl PolicyQueryExecutor for QueryPolicyExecutor<'_> {
    fn evaluate_policy_query<'b>(
        &'b self,
        query: &'b PolicyQuery,
        bindings: &'b HashMap<String, Sid>,
    ) -> PolicyQueryFut<'b> {
        Box::pin(self.evaluate_async(query, bindings))
    }
}

/// Map a JSON-LD special-variable name to its SPARQL registry name.
///
/// JSON-LD policy bindings use `?$this` / `?$identity`; SPARQL has no `$`
/// in variable names — the SHACL-SPARQL-style `$this` lexes as sigil `$` +
/// name `this` and registers as `?this`. So `?$this` maps to `?this`;
/// names without the `$` marker pass through unchanged.
fn sparql_var_name(json_ld_name: &str) -> String {
    match json_ld_name.strip_prefix("?$") {
        Some(rest) => format!("?{rest}"),
        None => json_ld_name.to_string(),
    }
}

impl QueryPolicyExecutor<'_> {
    /// Async implementation of policy query evaluation
    async fn evaluate_async(
        &self,
        query: &PolicyQuery,
        bindings: &HashMap<String, Sid>,
    ) -> PolicyResult<bool> {
        match query.language {
            PolicyQueryLanguage::JsonLd => self.evaluate_jsonld(&query.source, bindings).await,
            PolicyQueryLanguage::Sparql => self.evaluate_sparql(&query.source, bindings).await,
            // `PolicyQueryLanguage` is non_exhaustive; an unknown language
            // fails closed (error → deny), never open.
            other => Err(fluree_db_policy::PolicyError::QueryExecution {
                message: format!("Unsupported policy query language: {}", other.as_str()),
            }),
        }
    }

    /// Evaluate a JSON-LD policy query (the historical default).
    async fn evaluate_jsonld(
        &self,
        source: &str,
        bindings: &HashMap<String, Sid>,
    ) -> PolicyResult<bool> {
        // Parse and lower the policy's f:query using the main query parser/IR.
        //
        // We intentionally do NOT implement a bespoke parser here; this ensures full
        // feature parity (e.g., FILTER patterns) and avoids divergence.
        //
        // Policy queries behave like existence checks, with:
        // - select forced to ["?$this"]
        // - limit forced to 1
        // - VALUES injected into WHERE for special variables (?$this, ?$identity, etc.)
        let mut query_json: serde_json::Value = serde_json::from_str(source).map_err(|e| {
            fluree_db_policy::PolicyError::QueryExecution {
                message: format!("Invalid policy query JSON: {e}"),
            }
        })?;

        let obj = query_json.as_object_mut().ok_or_else(|| {
            fluree_db_policy::PolicyError::QueryExecution {
                message: "Policy query must be a JSON object".to_string(),
            }
        })?;

        // Force select + limit for policy queries
        obj.insert(
            "select".to_string(),
            serde_json::Value::Array(vec![serde_json::Value::String("?$this".to_string())]),
        );
        obj.insert("limit".to_string(), serde_json::Value::from(1));

        // Build VALUES clause JSON for special variables.
        // Inject VALUES into WHERE clause BEFORE parsing.
        // This ensures even empty queries (no WHERE) work - the VALUES provides the pattern.
        //
        // Format: ["values", [["?$this", "?$identity", ...], [[iri1, iri2, ...]]]]
        let mut var_names: Vec<String> = bindings.keys().cloned().collect();
        var_names.sort();

        // Build VALUES row with IRIs for each variable
        // Special case: unbound identity uses null (UNDEF) to ensure it never matches
        let values_row: Vec<serde_json::Value> = var_names
            .iter()
            .map(|name| {
                let sid = bindings.get(name).expect("binding value exists");
                // Check if this is an unbound identity - use null (UNDEF) instead of IRI
                // This ensures patterns referencing ?$identity won't match anything
                if sid.name.starts_with(UNBOUND_IDENTITY_PREFIX) {
                    return serde_json::Value::Null;
                }
                // Decode SID to IRI for JSON representation
                let iri = self
                    .snapshot
                    .decode_sid(sid)
                    .unwrap_or_else(|| sid.name.to_string());
                serde_json::json!({"@id": iri})
            })
            .collect();

        let values_clause = serde_json::json!(["values", [var_names.clone(), [values_row]]]);

        // Inject VALUES into WHERE clause (or create WHERE if missing)
        let where_clause = obj.get_mut("where");
        match where_clause {
            Some(serde_json::Value::Array(arr)) => {
                // WHERE is an array - prepend VALUES
                arr.insert(0, values_clause);
            }
            Some(serde_json::Value::Object(_)) => {
                // WHERE is an object (single pattern) - wrap in array with VALUES
                let existing = obj.remove("where").unwrap();
                obj.insert(
                    "where".to_string(),
                    serde_json::json!([values_clause, existing]),
                );
            }
            Some(_) | None => {
                // No WHERE or invalid - create with just VALUES
                // This handles empty queries like {}
                obj.insert("where".to_string(), serde_json::json!([values_clause]));
            }
        }

        // Create a variable registry for this query execution
        let mut vars = VarRegistry::new();

        // Pre-register special variables so they are present even if not referenced.
        // This matches the "always ground" behavior.
        for var_name in &var_names {
            vars.get_or_insert(var_name);
        }

        let parsed = crate::parse::parse_query(&query_json, self.snapshot, &mut vars, None)
            .map_err(|e| fluree_db_policy::PolicyError::QueryExecution {
                message: format!("Failed to parse policy query: {e}"),
            })?;

        self.run_existence_check(&vars, &parsed.patterns).await
    }

    /// Evaluate a SPARQL policy query (`f:query` stored with the `f:sparql`
    /// datatype).
    ///
    /// SPARQL support is provided by a higher layer via
    /// [`crate::lang_support::register_sparql_support`]; if it is absent this
    /// fails closed (error → deny), never open.
    async fn evaluate_sparql(
        &self,
        source: &str,
        bindings: &HashMap<String, Sid>,
    ) -> PolicyResult<bool> {
        let support = crate::lang_support::sparql_support().ok_or_else(|| {
            fluree_db_policy::PolicyError::QueryExecution {
                message: "SPARQL policy support is not registered in this process; \
                          cannot evaluate f:sparql policy query"
                    .to_string(),
            }
        })?;

        let mut vars = VarRegistry::new();
        let mut patterns =
            (support.lower_policy_query)(source, self.snapshot, &mut vars).map_err(|e| {
                fluree_db_policy::PolicyError::QueryExecution {
                    message: format!("Failed to parse SPARQL policy query: {e}"),
                }
            })?;

        // Seed special variables with a VALUES pattern, mirroring the JSON-LD
        // path's injected VALUES clause. Binding keys arrive in JSON-LD form
        // (`?$this`); the SPARQL query references them as `$this`/`?this`,
        // registered as `?this`.
        let mut var_names: Vec<String> = bindings.keys().cloned().collect();
        var_names.sort();

        let mut var_ids = Vec::with_capacity(var_names.len());
        let mut row = Vec::with_capacity(var_names.len());
        for name in &var_names {
            let var_id = vars.get_or_insert(&sparql_var_name(name));
            if var_ids.contains(&var_id) {
                continue;
            }
            let sid = bindings.get(name).expect("binding value exists");
            var_ids.push(var_id);
            // Unbound identity uses UNDEF (Binding::Unbound), same as the
            // JSON-LD path's null VALUES cell.
            if sid.name.starts_with(UNBOUND_IDENTITY_PREFIX) {
                row.push(Binding::Unbound);
            } else {
                row.push(Binding::Sid {
                    sid: sid.clone(),
                    t: None,
                    op: None,
                });
            }
        }
        patterns.insert(
            0,
            Pattern::Values {
                vars: var_ids,
                rows: vec![row],
            },
        );

        self.run_existence_check(&vars, &patterns).await
    }

    /// Execute WHERE patterns with a root (policy-free) context and report
    /// whether any solution exists.
    async fn run_existence_check(
        &self,
        vars: &VarRegistry,
        patterns: &[Pattern],
    ) -> PolicyResult<bool> {
        // Create the execution context WITHOUT policy (root context)
        // This is critical - policy queries must not be filtered by policy
        let ctx = if let Some(overlay) = self.overlay {
            ExecutionContext::with_time_and_overlay(self.snapshot, vars, self.to_t, None, overlay)
                .with_graph_id(self.g_id)
        } else {
            ExecutionContext::with_time(self.snapshot, vars, self.to_t, None)
                .with_graph_id(self.g_id)
        };

        // Build the where clause operators (VALUES is now part of the patterns).
        //
        // Root: policy queries always evaluate at `self.to_t` for current state —
        // they're access-control predicates, not history-range queries. Always
        // plan as `Current`.
        let mut operator = build_where_operators_seeded(
            None,
            patterns,
            None,
            None,
            &crate::temporal_mode::PlanningContext::current(),
        )
        .map_err(|e| fluree_db_policy::PolicyError::QueryExecution {
            message: e.to_string(),
        })?;

        // Execute and check if there's at least one result (existence check)
        operator
            .open(&ctx)
            .await
            .map_err(|e| fluree_db_policy::PolicyError::QueryExecution {
                message: e.to_string(),
            })?;

        let has_results = match operator.next_batch(&ctx).await {
            Ok(Some(batch)) => !batch.is_empty(),
            Ok(None) => false,
            Err(e) => {
                operator.close();
                return Err(fluree_db_policy::PolicyError::QueryExecution {
                    message: e.to_string(),
                });
            }
        };

        operator.close();

        Ok(has_results)
    }
}
