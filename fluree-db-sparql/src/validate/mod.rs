//! SPARQL Validation.
//!
//! This module validates parsed SPARQL AST against Fluree's capability model
//! and SPARQL semantic rules.
//!
//! ## Responsibilities
//!
//! - Fluree restrictions (property path depth, USING NAMED, etc.)
//! - Ground-only validation for INSERT DATA / DELETE DATA
//! - Variable scoping rules (future)
//!
//! ## Design
//!
//! Validation produces diagnostics without transforming the AST.
//! This enables standalone validation for IDE/LSP integration.
//!
//! ## Usage
//!
//! ```
//! use fluree_db_sparql::{parse_sparql, validate, Capabilities};
//!
//! let output = parse_sparql("SELECT ?x WHERE { ?x <http://example.org/p> ?y }");
//! if let Some(ast) = &output.ast {
//!     let diagnostics = validate(ast, &Capabilities::default());
//!     for d in &diagnostics {
//!         println!("{}: {}", d.code, d.message);
//!     }
//! }
//! ```

mod bnode_scope;
mod projection;

use crate::ast::path::PropertyPath;
use crate::ast::pattern::{GraphPattern, TriplePattern};
use crate::ast::query::{
    AskQuery, ConstructQuery, DescribeQuery, QueryBody, SelectQuery, SparqlAst,
};
use crate::ast::term::{PredicateTerm, SubjectTerm, Term};
use crate::ast::update::{
    DeleteData, DeleteWhere, InsertData, Modify, QuadData, QuadPattern, QuadPatternElement,
    UpdateOperation,
};
use crate::diag::{DiagCode, Diagnostic, Label};
use crate::span::SourceSpan;

/// Fluree capability configuration.
///
/// Controls which SPARQL features are allowed during validation.
/// By default, all Fluree-supported features are enabled.
#[derive(Clone, Debug)]
pub struct Capabilities {
    /// Allow property path operators (+, *, ?, /, |, ^)
    pub property_paths: bool,
    /// Allow MINUS operator (with partial semantics warning)
    pub minus_operator: bool,
    /// Allow USING clause in updates
    pub using_clause: bool,
}

impl Default for Capabilities {
    fn default() -> Self {
        Self {
            property_paths: true,
            minus_operator: true,
            using_clause: true,
        }
    }
}

/// Validate a SPARQL AST against capabilities and semantic rules.
///
/// Returns a list of diagnostics (errors and warnings).
/// An empty list indicates the query is valid.
pub fn validate(ast: &SparqlAst, caps: &Capabilities) -> Vec<Diagnostic> {
    let mut validator = Validator::new(caps);
    validator.validate_ast(ast);
    validator.diagnostics
}

/// Internal validator state.
struct Validator<'a> {
    caps: &'a Capabilities,
    diagnostics: Vec<Diagnostic>,
}

impl<'a> Validator<'a> {
    fn new(caps: &'a Capabilities) -> Self {
        Self {
            caps,
            diagnostics: Vec::new(),
        }
    }

    fn validate_ast(&mut self, ast: &SparqlAst) {
        match &ast.body {
            QueryBody::Select(query) => self.validate_select(query),
            QueryBody::Construct(query) => self.validate_construct(query),
            QueryBody::Ask(query) => self.validate_ask(query),
            QueryBody::Describe(query) => self.validate_describe(query),
            QueryBody::Update(op) => self.validate_update(op),
        }
    }

    fn validate_select(&mut self, query: &SelectQuery) {
        self.validate_query_where(&query.where_clause.pattern);
        projection::check_projection_scope(
            &query.select.variables,
            query.modifiers.group_by.as_ref(),
            query.select.span,
            &mut self.diagnostics,
        );
    }

    fn validate_construct(&mut self, query: &ConstructQuery) {
        self.validate_query_where(&query.where_clause.pattern);
        // Template triples don't need ground validation (they use WHERE variables)
    }

    fn validate_ask(&mut self, query: &AskQuery) {
        self.validate_query_where(&query.where_clause.pattern);
    }

    fn validate_describe(&mut self, query: &DescribeQuery) {
        if let Some(where_clause) = &query.where_clause {
            self.validate_query_where(&where_clause.pattern);
        }
    }

    /// Shared validation for a query form's WHERE pattern: the capability
    /// walk plus the query-only semantic passes (blank-node label scope —
    /// update operations have their own blank-node rules and are exempt).
    fn validate_query_where(&mut self, pattern: &GraphPattern) {
        self.validate_graph_pattern(pattern);
        bnode_scope::check_blank_node_scopes(pattern, &mut self.diagnostics);
    }

    fn validate_update(&mut self, op: &UpdateOperation) {
        match op {
            UpdateOperation::InsertData(insert) => {
                self.validate_insert_data(insert);
            }
            UpdateOperation::DeleteData(delete) => {
                self.validate_delete_data(delete);
            }
            UpdateOperation::DeleteWhere(delete_where) => {
                self.validate_delete_where(delete_where);
            }
            UpdateOperation::Modify(modify) => {
                self.validate_modify(modify);
            }
        }
    }

    /// Validate INSERT DATA - triples must be ground (no variables).
    fn validate_insert_data(&mut self, insert: &InsertData) {
        self.validate_ground_quad_data(&insert.data, "INSERT DATA");
    }

    /// Validate DELETE DATA - triples must be ground (no variables).
    fn validate_delete_data(&mut self, delete: &DeleteData) {
        self.validate_ground_quad_data(&delete.data, "DELETE DATA");
    }

    /// Validate DELETE WHERE - patterns can have variables.
    fn validate_delete_where(&mut self, delete_where: &DeleteWhere) {
        // DELETE WHERE allows variables - no ground validation needed.
        //
        // Phase 1: GRAPH blocks in DELETE WHERE are not supported yet because the lowering
        // path in `fluree-db-transact` currently targets triple-only patterns.
        for el in &delete_where.pattern.patterns {
            if let QuadPatternElement::Graph { span, .. } = el {
                self.diagnostics.push(
                    Diagnostic::error(
                        DiagCode::UnsupportedGraphInUpdate,
                        "GRAPH blocks are not supported in DELETE WHERE yet",
                        *span,
                    )
                    .with_help("Rewrite using explicit triples in the default graph, or use DELETE/INSERT with WHERE once GRAPH template support is extended to DELETE WHERE."),
                );
            }
        }
    }

    /// Validate Modify (INSERT/DELETE with WHERE).
    fn validate_modify(&mut self, modify: &Modify) {
        // DELETE and INSERT templates can have variables (bound by WHERE)
        // No ground validation needed for templates
        if let Some(delete_clause) = &modify.delete_clause {
            self.validate_update_template_quad_pattern(delete_clause, "DELETE");
        }
        if let Some(insert_clause) = &modify.insert_clause {
            self.validate_update_template_quad_pattern(insert_clause, "INSERT");
        }

        // Validate WHERE graph pattern (same capabilities as query WHERE).
        self.validate_graph_pattern(&modify.where_clause);
    }

    fn validate_update_template_quad_pattern(&mut self, pattern: &QuadPattern, context: &str) {
        for el in &pattern.patterns {
            if let QuadPatternElement::Graph { name, span, .. } = el {
                match name {
                    crate::ast::pattern::GraphName::Iri(_iri) => {
                        // Allowed (Phase 1)
                    }
                    crate::ast::pattern::GraphName::Var(v) => {
                        self.diagnostics.push(
                            Diagnostic::error(
                                DiagCode::UnsupportedGraphInUpdate,
                                format!(
                                    "GRAPH ?{} is not supported in SPARQL Update {} templates",
                                    v.name, context
                                ),
                                *span,
                            )
                            .with_label(Label::new(v.span, "graph variables not supported here"))
                            .with_help(
                                "Use GRAPH <iri> { ... } with an explicit graph IRI, or rewrite to a fixed target graph.",
                            ),
                        );
                    }
                }
            }
        }
    }

    /// Validate that QuadData contains only ground triples (no variables),
    /// including inside `GRAPH <iri> { ... }` blocks. Variable graph names are
    /// rejected: DATA must be ground.
    fn validate_ground_quad_data(&mut self, data: &QuadData, context: &str) {
        for el in &data.quads {
            match el {
                QuadPatternElement::Triple(triple) => {
                    self.validate_ground_triple(triple, context);
                }
                QuadPatternElement::Graph {
                    name,
                    triples,
                    span,
                } => {
                    if let crate::ast::pattern::GraphName::Var(v) = name {
                        self.diagnostics.push(
                            Diagnostic::error(
                                DiagCode::VariableInGroundData,
                                format!("Variable graph name ?{} not allowed in {context}", v.name),
                                *span,
                            )
                            .with_label(Label::new(v.span, "variable not allowed here"))
                            .with_help(format!(
                                "{context} requires a fixed graph IRI; use GRAPH <iri> {{ ... }}."
                            ))
                            .with_note(
                                "Use INSERT/DELETE with a WHERE clause for variable graph targets.",
                            ),
                        );
                    }
                    for triple in triples {
                        self.validate_ground_triple(triple, context);
                    }
                }
            }
        }
    }

    /// Validate that a triple pattern is ground (no variables).
    fn validate_ground_triple(&mut self, triple: &TriplePattern, context: &str) {
        // Check subject
        if let SubjectTerm::Var(var) = &triple.subject {
            self.diagnostics.push(
                Diagnostic::error(
                    DiagCode::VariableInGroundData,
                    format!("Variable ?{} not allowed in {}", var.name, context),
                    var.span,
                )
                .with_label(Label::new(var.span, "variable not allowed here"))
                .with_help(format!(
                    "{context} requires ground triples (IRIs, literals, blank nodes) with no variables."
                ))
                .with_note(
                    "Use DELETE WHERE or INSERT/DELETE with WHERE clause for patterns with variables.",
                ),
            );
        }

        // Check predicate
        if let PredicateTerm::Var(var) = &triple.predicate {
            self.diagnostics.push(
                Diagnostic::error(
                    DiagCode::VariableInGroundData,
                    format!("Variable ?{} not allowed in {}", var.name, context),
                    var.span,
                )
                .with_label(Label::new(var.span, "variable not allowed here"))
                .with_help(format!(
                    "{context} requires ground triples (IRIs, literals, blank nodes) with no variables."
                ))
                .with_note(
                    "Use DELETE WHERE or INSERT/DELETE with WHERE clause for patterns with variables.",
                ),
            );
        }

        // Check object
        if let Term::Var(var) = &triple.object {
            self.diagnostics.push(
                Diagnostic::error(
                    DiagCode::VariableInGroundData,
                    format!("Variable ?{} not allowed in {}", var.name, context),
                    var.span,
                )
                .with_label(Label::new(var.span, "variable not allowed here"))
                .with_help(format!(
                    "{context} requires ground triples (IRIs, literals, blank nodes) with no variables."
                ))
                .with_note(
                    "Use DELETE WHERE or INSERT/DELETE with WHERE clause for patterns with variables.",
                ),
            );
        }
    }

    /// Validate a graph pattern recursively.
    fn validate_graph_pattern(&mut self, pattern: &GraphPattern) {
        match pattern {
            GraphPattern::Bgp { patterns, .. } => {
                for triple in patterns {
                    self.validate_triple_pattern(triple);
                }
            }
            GraphPattern::Group { patterns, .. } => {
                for p in patterns {
                    self.validate_graph_pattern(p);
                }
            }
            GraphPattern::Optional { pattern, .. } => {
                self.validate_graph_pattern(pattern);
            }
            GraphPattern::Union { left, right, .. } => {
                self.validate_graph_pattern(left);
                self.validate_graph_pattern(right);
            }
            GraphPattern::Minus { left, right, span } => {
                self.validate_graph_pattern(left);
                self.validate_graph_pattern(right);
                // Emit warning about partial MINUS semantics
                if self.caps.minus_operator {
                    self.diagnostics.push(
                        Diagnostic::warning(
                            DiagCode::MinusSemanticsPartial,
                            "MINUS may have different semantics than SPARQL specification",
                            *span,
                        )
                        .with_note(
                            "Fluree's MINUS implementation may differ from standard SPARQL \
                             in edge cases involving unbound variables.",
                        ),
                    );
                }
            }
            GraphPattern::Filter { .. } => {
                // Expression validation could be added here
            }
            GraphPattern::Bind { .. } => {
                // Expression validation could be added here
            }
            GraphPattern::Values { .. } => {
                // Values are ground by construction
            }
            GraphPattern::Graph { pattern, .. } => {
                self.validate_graph_pattern(pattern);
            }
            GraphPattern::Service { pattern, .. } => {
                self.validate_graph_pattern(pattern);
            }
            GraphPattern::SubSelect { query, span } => {
                self.validate_graph_pattern(&query.pattern);
                projection::check_projection_scope(
                    &query.variables,
                    query.modifiers.group_by.as_ref(),
                    *span,
                    &mut self.diagnostics,
                );
            }
            GraphPattern::Path { path, span, .. } => {
                self.validate_property_path(path, *span);
            }
            GraphPattern::AnnotationTarget { .. } => {
                // No additional validation here; deferred-shape rejection
                // happens at parse time and lowering enforces the rest.
            }
        }
    }

    /// Validate a triple pattern for property paths.
    fn validate_triple_pattern(&mut self, _triple: &TriplePattern) {
        // Triple patterns use PredicateTerm (Var or Iri), not PropertyPath
        // No property path validation needed for basic triples
    }

    /// Validate a property path for unsupported features.
    fn validate_property_path(&mut self, path: &PropertyPath, _pattern_span: SourceSpan) {
        match path {
            // Negated property sets lower to a fresh-predicate-var triple plus a
            // FILTER excluding the listed predicates (see lower/path.rs). Members
            // are leaf IRIs/`a` (optionally inverse), so no inner recursion.
            PropertyPath::NegatedSet { .. } => {}
            PropertyPath::Iri(_) | PropertyPath::A { .. } => {
                // Simple paths are always valid
            }
            PropertyPath::Inverse { path, .. } => {
                self.validate_property_path(path, path.span());
            }
            PropertyPath::Sequence { left, right, .. } => {
                self.validate_property_path(left, left.span());
                self.validate_property_path(right, right.span());
            }
            PropertyPath::Alternative { left, right, .. } => {
                self.validate_property_path(left, left.span());
                self.validate_property_path(right, right.span());
            }
            PropertyPath::ZeroOrMore { path, .. } => {
                self.validate_property_path(path, path.span());
            }
            PropertyPath::OneOrMore { path, .. } => {
                self.validate_property_path(path, path.span());
            }
            PropertyPath::ZeroOrOne { path, .. } => {
                self.validate_property_path(path, path.span());
            }
            PropertyPath::Group { path, .. } => {
                self.validate_property_path(path, path.span());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_sparql;

    fn validate_query(sparql: &str) -> Vec<Diagnostic> {
        let output = parse_sparql(sparql);
        assert!(
            output.ast.is_some(),
            "Parse failed: {:?}",
            output.diagnostics
        );
        validate(output.ast.as_ref().unwrap(), &Capabilities::default())
    }

    // =========================================================================
    // Ground-only validation tests (INSERT DATA / DELETE DATA)
    // =========================================================================

    #[test]
    fn test_insert_data_ground_valid() {
        let diags = validate_query(
            "INSERT DATA { <http://example.org/s> <http://example.org/p> \"value\" }",
        );
        assert!(
            diags.iter().all(|d| !d.is_error()),
            "Expected no errors: {diags:?}"
        );
    }

    #[test]
    fn test_insert_data_variable_subject() {
        let diags = validate_query("INSERT DATA { ?s <http://example.org/p> \"value\" }");
        assert!(diags
            .iter()
            .any(|d| d.code == DiagCode::VariableInGroundData));
    }

    #[test]
    fn test_insert_data_variable_predicate() {
        let diags = validate_query("INSERT DATA { <http://example.org/s> ?p \"value\" }");
        assert!(diags
            .iter()
            .any(|d| d.code == DiagCode::VariableInGroundData));
    }

    #[test]
    fn test_insert_data_variable_object() {
        let diags =
            validate_query("INSERT DATA { <http://example.org/s> <http://example.org/p> ?o }");
        assert!(diags
            .iter()
            .any(|d| d.code == DiagCode::VariableInGroundData));
    }

    #[test]
    fn test_insert_data_all_variables() {
        let diags = validate_query("INSERT DATA { ?s ?p ?o }");
        // Should have 3 errors (one per variable position)
        let var_errors: Vec<_> = diags
            .iter()
            .filter(|d| d.code == DiagCode::VariableInGroundData)
            .collect();
        assert_eq!(var_errors.len(), 3);
    }

    #[test]
    fn test_delete_data_ground_valid() {
        let diags = validate_query(
            "DELETE DATA { <http://example.org/s> <http://example.org/p> \"value\" }",
        );
        assert!(
            diags.iter().all(|d| !d.is_error()),
            "Expected no errors: {diags:?}"
        );
    }

    #[test]
    fn test_delete_data_variable() {
        let diags = validate_query("DELETE DATA { ?s <http://example.org/p> \"value\" }");
        assert!(diags
            .iter()
            .any(|d| d.code == DiagCode::VariableInGroundData));
    }

    #[test]
    fn test_insert_data_graph_block_ground_valid() {
        // Issue #1288: GRAPH <iri> { ground triples } is valid in INSERT DATA.
        let diags = validate_query(
            "INSERT DATA { GRAPH <urn:g> { <http://example.org/s> <http://example.org/p> \"v\" } }",
        );
        assert!(
            diags.iter().all(|d| !d.is_error()),
            "Expected no errors: {diags:?}"
        );
    }

    #[test]
    fn test_insert_data_graph_block_variable_rejected() {
        // A variable inside a GRAPH block in DATA is still non-ground.
        let diags =
            validate_query("INSERT DATA { GRAPH <urn:g> { ?s <http://example.org/p> \"v\" } }");
        assert!(diags
            .iter()
            .any(|d| d.code == DiagCode::VariableInGroundData));
    }

    #[test]
    fn test_insert_data_variable_graph_name_rejected() {
        // A variable graph name is not ground and must be rejected.
        let diags = validate_query(
            "INSERT DATA { GRAPH ?g { <http://example.org/s> <http://example.org/p> \"v\" } }",
        );
        assert!(diags
            .iter()
            .any(|d| d.code == DiagCode::VariableInGroundData));
    }

    // =========================================================================
    // DELETE WHERE and Modify tests (variables allowed)
    // =========================================================================

    #[test]
    fn test_delete_where_variables_allowed() {
        let diags = validate_query("DELETE WHERE { ?s ?p ?o }");
        // Variables are allowed in DELETE WHERE
        assert!(
            !diags
                .iter()
                .any(|d| d.code == DiagCode::VariableInGroundData),
            "Variables should be allowed in DELETE WHERE"
        );
    }

    #[test]
    fn test_modify_variables_allowed() {
        let diags = validate_query(
            "DELETE { ?s ex:old ?o } INSERT { ?s ex:new ?o } WHERE { ?s ex:old ?o }",
        );
        // Variables are allowed in DELETE/INSERT with WHERE
        assert!(
            !diags
                .iter()
                .any(|d| d.code == DiagCode::VariableInGroundData),
            "Variables should be allowed in Modify operations"
        );
    }

    // =========================================================================
    // Property path validation tests
    // =========================================================================

    #[test]
    fn test_property_path_simple_valid() {
        let diags = validate_query("SELECT * WHERE { ?s ex:name ?o }");
        assert!(
            diags.iter().all(|d| !d.is_error()),
            "Expected no errors: {diags:?}"
        );
    }

    #[test]
    fn test_property_path_transitive_valid() {
        let diags = validate_query("SELECT * WHERE { ?s ex:parent+ ?o }");
        assert!(
            !diags
                .iter()
                .any(|d| d.code == DiagCode::UnsupportedNegatedPropertySet),
            "Transitive paths should be valid"
        );
    }

    #[test]
    fn test_property_path_negated_now_valid() {
        // Negated property sets are now supported (lowered to a fresh-predicate
        // triple + FILTER); the validator no longer rejects them.
        let diags = validate_query("SELECT * WHERE { ?s !ex:hidden ?o }");
        assert!(
            !diags
                .iter()
                .any(|d| d.code == DiagCode::UnsupportedNegatedPropertySet),
            "Negated property sets are supported and should validate"
        );
    }

    #[test]
    fn test_property_path_negated_set_now_valid() {
        let diags = validate_query("SELECT * WHERE { ?s !(ex:a|ex:b) ?o }");
        assert!(
            !diags
                .iter()
                .any(|d| d.code == DiagCode::UnsupportedNegatedPropertySet),
            "Negated property sets are supported and should validate"
        );
    }

    #[test]
    fn test_property_path_complex_valid() {
        let diags = validate_query("SELECT * WHERE { ?s ^ex:parent/ex:child+ ?o }");
        assert!(
            !diags
                .iter()
                .any(|d| d.code == DiagCode::UnsupportedNegatedPropertySet),
            "Complex but supported paths should be valid"
        );
    }

    // =========================================================================
    // USING clause validation tests
    // =========================================================================

    #[test]
    fn test_using_clause_valid() {
        let diags = validate_query(
            "DELETE { ?s ex:p ?o } WHERE { ?s ex:p ?o } USING <http://example.org/graph>",
        );
        assert!(
            !diags
                .iter()
                .any(|d| d.code == DiagCode::UnsupportedUsingNamed),
            "USING should be valid"
        );
    }

    // Note: USING NAMED parsing would need to be tested if we add parser support
    // Currently the parser may not parse USING NAMED syntax

    // =========================================================================
    // MINUS warning tests
    // =========================================================================

    #[test]
    fn test_minus_warning() {
        // MINUS must be inside the WHERE clause braces
        let diags = validate_query("SELECT * WHERE { ?s ?p ?o MINUS { ?s ex:hidden ?o } }");
        assert!(
            diags
                .iter()
                .any(|d| d.code == DiagCode::MinusSemanticsPartial && d.is_warning()),
            "MINUS should emit a warning: {diags:?}"
        );
    }

    // =========================================================================
    // SELECT query tests (no special validation needed)
    // =========================================================================

    #[test]
    fn test_select_query_valid() {
        let diags = validate_query("SELECT ?x ?y WHERE { ?x ex:knows ?y }");
        assert!(
            diags.iter().all(|d| !d.is_error()),
            "Expected no errors: {diags:?}"
        );
    }

    // =========================================================================
    // V3 — blank-node label scope tests (SPARQL 1.1 §19.6)
    // =========================================================================

    fn has_code(diags: &[Diagnostic], code: DiagCode) -> bool {
        diags.iter().any(|d| d.code == code && d.is_error())
    }

    #[test]
    fn test_bnode_scope_same_bgp_valid() {
        // Same label twice in one basic graph pattern is fine.
        let diags = validate_query("SELECT * WHERE { _:a ex:p ?v . _:a ex:q 1 }");
        assert!(
            !has_code(&diags, DiagCode::BlankNodeLabelCrossScope),
            "{diags:?}"
        );
    }

    #[test]
    fn test_bnode_scope_across_filter_valid() {
        // FILTER does not end a basic graph pattern (§18.2.2.5).
        let diags = validate_query("SELECT * WHERE { _:a ex:p ?v FILTER(?v > 1) _:a ex:q 1 }");
        assert!(
            !has_code(&diags, DiagCode::BlankNodeLabelCrossScope),
            "{diags:?}"
        );
    }

    #[test]
    fn test_bnode_scope_across_optional_rejected() {
        let diags = validate_query("SELECT * WHERE { _:a ex:p ?v OPTIONAL { _:a ex:q 1 } }");
        assert!(has_code(&diags, DiagCode::BlankNodeLabelCrossScope));
    }

    #[test]
    fn test_bnode_scope_across_nested_group_rejected() {
        let diags = validate_query("SELECT * WHERE { _:a ?p ?v . { _:a ?q 1 } }");
        assert!(has_code(&diags, DiagCode::BlankNodeLabelCrossScope));
    }

    #[test]
    fn test_bnode_scope_across_union_rejected() {
        let diags =
            validate_query("SELECT * WHERE { { _:a ex:p ?v } UNION { _:a ex:q 1 } }");
        assert!(has_code(&diags, DiagCode::BlankNodeLabelCrossScope));
    }

    #[test]
    fn test_bnode_scope_boundary_breaks_bgp_rejected() {
        // Reuse in the SAME group but across a GRAPH boundary: the GRAPH
        // pattern ends the first BGP, so the second `_:a` is a new BGP.
        let diags = validate_query(
            "SELECT * WHERE { _:a ?p ?v . GRAPH ?g { ?s ?p ?v } _:a ?q 1 }",
        );
        assert!(has_code(&diags, DiagCode::BlankNodeLabelCrossScope));
    }

    #[test]
    fn test_bnode_scope_distinct_labels_valid() {
        let diags =
            validate_query("SELECT * WHERE { _:a ex:p ?v OPTIONAL { _:b ex:q 1 } }");
        assert!(
            !has_code(&diags, DiagCode::BlankNodeLabelCrossScope),
            "{diags:?}"
        );
    }

    #[test]
    fn test_bnode_scope_anon_bnodes_valid() {
        // `[]` anonymous blank nodes have no label and are never flagged.
        let diags = validate_query("SELECT * WHERE { [] ex:p ?v OPTIONAL { [] ex:q 1 } }");
        assert!(
            !has_code(&diags, DiagCode::BlankNodeLabelCrossScope),
            "{diags:?}"
        );
    }

    // =========================================================================
    // V4 — GROUP BY / aggregate projection-scope tests (SPARQL 1.1 §11)
    // =========================================================================

    #[test]
    fn test_projection_star_with_group_by_rejected() {
        let diags = validate_query("SELECT * { ?s ?p ?o } GROUP BY ?s");
        assert!(has_code(&diags, DiagCode::SelectStarWithGroupBy));
    }

    #[test]
    fn test_projection_ungrouped_var_rejected() {
        let diags = validate_query("SELECT ?o { ?s ?p ?o } GROUP BY ?s");
        assert!(has_code(&diags, DiagCode::UngroupedVariableInProjection));
    }

    #[test]
    fn test_projection_group_key_and_aggregate_valid() {
        let diags = validate_query(
            "SELECT ?s (COUNT(?o) AS ?c) WHERE { ?s ?p ?o } GROUP BY ?s",
        );
        assert!(
            !has_code(&diags, DiagCode::UngroupedVariableInProjection),
            "{diags:?}"
        );
    }

    #[test]
    fn test_projection_implicit_group_ungrouped_var_rejected() {
        // agg10 shape: an aggregate in the projection groups implicitly;
        // ?p is then neither a key nor aggregated.
        let diags = validate_query("SELECT ?p (COUNT(?o) AS ?c) WHERE { ?s ?p ?o }");
        assert!(has_code(&diags, DiagCode::UngroupedVariableInProjection));
    }

    #[test]
    fn test_projection_expression_key_vars_not_licensed() {
        // agg08 shape: GROUP BY (?a + ?b) — the *expression* is the key,
        // not its variables.
        let diags = validate_query(
            "SELECT ((?a + ?b) AS ?ab) (COUNT(?a) AS ?c) WHERE { ?s ?p ?a, ?b } GROUP BY (?a + ?b)",
        );
        assert!(has_code(&diags, DiagCode::UngroupedVariableInProjection));
    }

    #[test]
    fn test_projection_bracketed_var_key_valid() {
        // GROUP BY (?s) — a bracketed bare variable counts as the key ?s.
        let diags = validate_query(
            "SELECT ?s (COUNT(?o) AS ?c) WHERE { ?s ?p ?o } GROUP BY (?s)",
        );
        assert!(
            !has_code(&diags, DiagCode::UngroupedVariableInProjection),
            "{diags:?}"
        );
    }

    #[test]
    fn test_projection_group_by_alias_valid() {
        let diags = validate_query(
            "SELECT ?key (COUNT(?o) AS ?c) WHERE { ?s ?p ?o } GROUP BY (STR(?s) AS ?key)",
        );
        assert!(
            !has_code(&diags, DiagCode::UngroupedVariableInProjection),
            "{diags:?}"
        );
    }

    #[test]
    fn test_projection_earlier_alias_usable_in_later_expression() {
        let diags = validate_query(
            "SELECT (SUM(?x) AS ?sum) ((?sum + 1) AS ?sump) WHERE { ?s ?p ?x } GROUP BY ?s",
        );
        assert!(
            !has_code(&diags, DiagCode::UngroupedVariableInProjection),
            "{diags:?}"
        );
    }

    #[test]
    fn test_projection_ungrouped_query_unaffected() {
        let diags = validate_query("SELECT ?s ?o WHERE { ?s ?p ?o }");
        assert!(
            !has_code(&diags, DiagCode::UngroupedVariableInProjection)
                && !has_code(&diags, DiagCode::SelectStarWithGroupBy),
            "{diags:?}"
        );
    }

    #[test]
    fn test_projection_subselect_checked() {
        let diags = validate_query(
            "SELECT ?s WHERE { { SELECT ?o { ?s ?p ?o } GROUP BY ?s } ?s ?p ?o2 }",
        );
        assert!(has_code(&diags, DiagCode::UngroupedVariableInProjection));
    }

    // =========================================================================
    // Diagnostic message quality tests
    // =========================================================================

    #[test]
    fn test_variable_error_has_help() {
        let diags = validate_query("INSERT DATA { ?s <http://example.org/p> \"value\" }");
        let var_error = diags
            .iter()
            .find(|d| d.code == DiagCode::VariableInGroundData)
            .expect("Expected variable error");
        assert!(var_error.help.is_some(), "Error should have help text");
        assert!(var_error.note.is_some(), "Error should have a note");
    }
}
