//! Per-suite skip registers for the W3C SPARQL test suites.
//!
//! Each entry is a test that currently FAILS for a known reason. The suites
//! themselves always run in CI; `check_testsuite` enforces this register in
//! BOTH directions:
//!
//! - a test that fails and is not listed here fails the suite (regression);
//! - a test that passes but IS listed here fails the suite (stale entry —
//!   remove it in the same change that fixes the feature).
//!
//! Grouping comments name the root cause. The full failure taxonomy, root
//! causes, and burn-down plan live in
//! `docs/audit/2026-07-sparql-testsuite-audit.md`; policy for adding entries
//! is in `docs/contributing/sparql-compliance.md` ("Managing the Skip List").
//! Baseline: rdf-tests submodule @ efccbc6b8, 2026-07-06.

pub const SPARQL11_SYNTAX_QUERY: &[&str] = &[
    // parser rejects valid input (4)
    // (test_21/test_23/test_64 were greened by main's sub-SELECT set-operand
    // fix, PR #1436)
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-query/manifest#test_35a",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-query/manifest#test_36a",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-query/manifest#test_63",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-query/manifest#test_pp_coll",
    // parser accepts invalid input (missing validation) (7)
    // (test_65 formerly failed to parse for the wrong reason; PR #1436 made
    // it parse, so it now needs the SELECT-scope validation pass — burn-down
    // PR-2 V4/V5 territory)
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-query/manifest#test_43",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-query/manifest#test_44",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-query/manifest#test_45",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-query/manifest#test_60",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-query/manifest#test_61a",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-query/manifest#test_62a",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-query/manifest#test_65",
];

// Dominated by the missing UPDATE graph-management grammar
// (LOAD/CLEAR/CREATE/DROP/COPY/MOVE/ADD, SILENT variants) — audit §4.2.1.
pub const SPARQL11_SYNTAX_UPDATE_1: &[&str] = &[
    // parser rejects valid input (27)
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_1",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_10",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_11",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_12",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_13",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_14",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_15",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_16",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_17",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_18",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_19",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_2",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_20",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_21",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_22",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_3",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_36",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_37",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_38",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_39",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_4",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_40",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_5",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_6",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_7",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_8",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_9",
    // parser accepts invalid input (missing validation) (4)
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_50",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_51",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_52",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_54",
];

pub const SPARQL10_SYNTAX: &[&str] = &[
    // parser accepts invalid input (missing validation) (24)
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/syntax-sparql3/manifest#blabel-cross-graph-bad",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/syntax-sparql3/manifest#blabel-cross-optional-bad",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/syntax-sparql3/manifest#blabel-cross-union-bad",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/syntax-sparql3/manifest#filter-missing-parens",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/syntax-sparql3/manifest#syn-bad-02",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/syntax-sparql3/manifest#syn-bad-03",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/syntax-sparql3/manifest#syn-bad-05",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/syntax-sparql3/manifest#syn-bad-06",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/syntax-sparql3/manifest#syn-bad-07",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/syntax-sparql3/manifest#syn-bad-08",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/syntax-sparql3/manifest#syn-bad-09",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/syntax-sparql3/manifest#syn-bad-10",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/syntax-sparql3/manifest#syn-bad-11",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/syntax-sparql3/manifest#syn-bad-12",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/syntax-sparql3/manifest#syn-bad-13",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/syntax-sparql3/manifest#syn-bad-14",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/syntax-sparql4/manifest#syn-bad-34",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/syntax-sparql4/manifest#syn-bad-35",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/syntax-sparql4/manifest#syn-bad-36",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/syntax-sparql4/manifest#syn-bad-37",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/syntax-sparql4/manifest#syn-bad-38",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/syntax-sparql4/manifest#syn-bad-GRAPH-breaks-BGP",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/syntax-sparql4/manifest#syn-bad-OPT-breaks-BGP",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/syntax-sparql4/manifest#syn-bad-UNION-breaks-BGP",
    // parser rejects valid input (17)
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/syntax-sparql1/manifest#syntax-forms-01",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/syntax-sparql1/manifest#syntax-forms-02",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/syntax-sparql1/manifest#syntax-lists-01",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/syntax-sparql1/manifest#syntax-lists-02",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/syntax-sparql1/manifest#syntax-lists-03",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/syntax-sparql1/manifest#syntax-lists-04",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/syntax-sparql1/manifest#syntax-lists-05",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/syntax-sparql1/manifest#syntax-order-07",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/syntax-sparql1/manifest#syntax-qname-05",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/syntax-sparql2/manifest#syntax-function-01",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/syntax-sparql2/manifest#syntax-function-02",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/syntax-sparql2/manifest#syntax-function-03",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/syntax-sparql2/manifest#syntax-lists-01",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/syntax-sparql2/manifest#syntax-lists-02",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/syntax-sparql2/manifest#syntax-lists-03",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/syntax-sparql2/manifest#syntax-lists-04",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/syntax-sparql2/manifest#syntax-lists-05",
];

pub const SPARQL11_AGGREGATES: &[&str] = &[
    // parser accepts invalid input (missing validation) (5)
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/aggregates/manifest#agg08",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/aggregates/manifest#agg09",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/aggregates/manifest#agg10",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/aggregates/manifest#agg11",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/aggregates/manifest#agg12",
    // result mismatch (3)
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/aggregates/manifest#agg-empty-group-count-graph",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/aggregates/manifest#agg-err-01",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/aggregates/manifest#agg02",
    // query execution error (1)
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/aggregates/manifest#agg-count-rows-distinct",
];

pub const SPARQL11_BINDINGS: &[&str] = &[
    // result mismatch (1)
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/bindings/manifest#graph",
];

pub const SPARQL11_CONSTRUCT: &[&str] = &[
    // dataset: FROM/FROM NAMED unsupported on single-ledger GraphDb (1)
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/construct/manifest#constructwhere04",
    // query execution error (1)
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/construct/manifest#constructlist",
];

pub const SPARQL11_EXISTS: &[&str] = &[
    // result mismatch (2)
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/exists/manifest#exists-graph-variable",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/exists/manifest#exists03",
];

pub const SPARQL11_FUNCTIONS: &[&str] = &[
    // result mismatch (6)
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/functions/manifest#bnode01",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/functions/manifest#concat02",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/functions/manifest#in01",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/functions/manifest#iri01",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/functions/manifest#notin01",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/functions/manifest#strlang03-rdf11",
];

pub const SPARQL11_GROUPING: &[&str] = &[
    // parser accepts invalid input (missing validation) (2)
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/grouping/manifest#group06",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/grouping/manifest#group07",
];

pub const SPARQL11_PROJECT_EXPRESSION: &[&str] = &[
    // result mismatch (1)
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/project-expression/manifest#projexp05",
];

pub const SPARQL11_SUBQUERY: &[&str] = &[
    // result mismatch (2)
    // (subquery12 was greened by main's sub-SELECT set-operand fix, PR #1436
    // — its "mismatch" was a bare-{SELECT} misparse executing through error
    // recovery)
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/subquery/manifest#subquery02",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/subquery/manifest#subquery04",
];

// Largest clusters: XSD type-promotion comparisons, open-world equality,
// expression builtins, FROM/FROM NAMED dataset construction, GRAPH ?g
// binding typed as literal — audit §4.2.
pub const SPARQL10_QUERY_EVAL: &[&str] = &[
    // result mismatch (113)
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/algebra/manifest#filter-nested-2",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/algebra/manifest#join-combo-2",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/algebra/manifest#join-scope-1",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/algebra/manifest#nested-opt-1",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/algebra/manifest#nested-opt-2",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/basic/manifest#base-prefix-2",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/basic/manifest#base-prefix-5",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/basic/manifest#list-1",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/basic/manifest#list-2",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/basic/manifest#list-3",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/basic/manifest#list-4",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/basic/manifest#quotes-3",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/basic/manifest#quotes-4",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/boolean-effective-value/manifest#dawg-bev-1",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/boolean-effective-value/manifest#dawg-bev-2",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/boolean-effective-value/manifest#dawg-bev-3",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/boolean-effective-value/manifest#dawg-bev-4",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/boolean-effective-value/manifest#dawg-bev-5",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/boolean-effective-value/manifest#dawg-bev-6",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/boolean-effective-value/manifest#dawg-boolean-literal",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/cast/manifest#cast-bool",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/cast/manifest#cast-dT",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/cast/manifest#cast-dbl",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/cast/manifest#cast-dec",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/cast/manifest#cast-flt",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/cast/manifest#cast-int",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/cast/manifest#cast-str",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/construct/manifest#construct-3",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/construct/manifest#construct-4",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/distinct/manifest#distinct-1",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/distinct/manifest#distinct-9",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/expr-builtin/manifest#dawg-datatype-2",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/expr-builtin/manifest#dawg-lang-1",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/expr-builtin/manifest#dawg-lang-2",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/expr-builtin/manifest#dawg-lang-3",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/expr-builtin/manifest#dawg-langMatches-1",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/expr-builtin/manifest#dawg-langMatches-2",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/expr-builtin/manifest#dawg-langMatches-3",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/expr-builtin/manifest#dawg-langMatches-4",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/expr-builtin/manifest#dawg-langMatches-basic",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/expr-builtin/manifest#dawg-str-1",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/expr-builtin/manifest#dawg-str-2",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/expr-builtin/manifest#sameTerm-eq",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/expr-builtin/manifest#sameTerm-not-eq",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/expr-builtin/manifest#sameTerm-simple",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/expr-equals/manifest#eq-2-1",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/expr-equals/manifest#eq-2-2",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/expr-equals/manifest#eq-4",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/expr-equals/manifest#eq-dateTime",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/expr-equals/manifest#eq-graph-1",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/expr-equals/manifest#eq-graph-2",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/expr-equals/manifest#eq-graph-4",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/expr-ops/manifest#add-literals",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/expr-ops/manifest#add-numbers-cast",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/expr-ops/manifest#divide-numbers-cast",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/expr-ops/manifest#multiply-numbers-cast",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/expr-ops/manifest#subtract-numbers-cast",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/expr-ops/manifest#unminus-2",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/expr-ops/manifest#unplus-2",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/graph/manifest#dawg-graph-03",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/graph/manifest#dawg-graph-04",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/graph/manifest#dawg-graph-06",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/graph/manifest#dawg-graph-07",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/graph/manifest#dawg-graph-08",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/graph/manifest#dawg-graph-09",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/graph/manifest#dawg-graph-10b",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/graph/manifest#dawg-graph-11",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/graph/manifest#graph-empty",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/graph/manifest#graph-exist",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/graph/manifest#graph-optional",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/graph/manifest#graph-variable-join",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/open-world/manifest#date-1",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/open-world/manifest#open-eq-01",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/open-world/manifest#open-eq-02",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/open-world/manifest#open-eq-04",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/open-world/manifest#open-eq-05",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/open-world/manifest#open-eq-06",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/open-world/manifest#open-eq-07",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/open-world/manifest#open-eq-08",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/open-world/manifest#open-eq-10",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/open-world/manifest#open-eq-11",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/open-world/manifest#open-eq-12",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/optional-filter/manifest#dawg-optional-filter-005-not-simplified",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/optional/manifest#dawg-optional-complex-2",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/optional/manifest#dawg-optional-complex-3",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/optional/manifest#dawg-optional-complex-4",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/regex/manifest#regex-no-metacharacters",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/regex/manifest#regex-no-metacharacters-case-insensitive",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/type-promotion/manifest#type-promotion-01",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/type-promotion/manifest#type-promotion-02",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/type-promotion/manifest#type-promotion-03",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/type-promotion/manifest#type-promotion-04",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/type-promotion/manifest#type-promotion-05",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/type-promotion/manifest#type-promotion-06",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/type-promotion/manifest#type-promotion-07",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/type-promotion/manifest#type-promotion-08",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/type-promotion/manifest#type-promotion-09",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/type-promotion/manifest#type-promotion-10",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/type-promotion/manifest#type-promotion-11",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/type-promotion/manifest#type-promotion-12",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/type-promotion/manifest#type-promotion-13",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/type-promotion/manifest#type-promotion-14",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/type-promotion/manifest#type-promotion-15",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/type-promotion/manifest#type-promotion-16",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/type-promotion/manifest#type-promotion-17",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/type-promotion/manifest#type-promotion-18",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/type-promotion/manifest#type-promotion-19",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/type-promotion/manifest#type-promotion-20",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/type-promotion/manifest#type-promotion-21",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/type-promotion/manifest#type-promotion-22",
    // dataset: FROM/FROM NAMED unsupported on single-ledger GraphDb (12)
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/dataset/manifest#dawg-dataset-01",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/dataset/manifest#dawg-dataset-02",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/dataset/manifest#dawg-dataset-03",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/dataset/manifest#dawg-dataset-04",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/dataset/manifest#dawg-dataset-05",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/dataset/manifest#dawg-dataset-06",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/dataset/manifest#dawg-dataset-07",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/dataset/manifest#dawg-dataset-08",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/dataset/manifest#dawg-dataset-09b",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/dataset/manifest#dawg-dataset-10b",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/dataset/manifest#dawg-dataset-11",
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/dataset/manifest#dawg-dataset-12b",
    // query execution error (1)
    "http://www.w3.org/2001/sw/DataAccess/tests/data-r2/basic/manifest#base-prefix-1",
];

// Clusters: graph-management ops missing from the UPDATE grammar,
// GRAPH blocks in DELETE WHERE, USING semantics, INSERT into
// not-yet-existing named graph — audit §4.2.1.
pub const SPARQL11_UPDATE: &[&str] = &[
    // update grammar: graph-management op not parsed (41)
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/add/manifest#add01",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/add/manifest#add02",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/add/manifest#add03",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/add/manifest#add04",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/add/manifest#add05",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/add/manifest#add06",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/add/manifest#add07",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/add/manifest#add08",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/clear/manifest#dawg-clear-all-01",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/clear/manifest#dawg-clear-default-01",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/clear/manifest#dawg-clear-graph-01",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/clear/manifest#dawg-clear-named-01",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/copy/manifest#copy01",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/copy/manifest#copy02",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/copy/manifest#copy03",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/copy/manifest#copy04",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/copy/manifest#copy06",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/copy/manifest#copy07",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/drop/manifest#dawg-drop-all-01",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/drop/manifest#dawg-drop-default-01",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/drop/manifest#dawg-drop-graph-01",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/drop/manifest#dawg-drop-named-01",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/move/manifest#move01",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/move/manifest#move02",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/move/manifest#move03",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/move/manifest#move04",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/move/manifest#move06",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/move/manifest#move07",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/update-silent/manifest#add-silent",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/update-silent/manifest#add-to-default-silent",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/update-silent/manifest#clear-default-silent",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/update-silent/manifest#clear-silent",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/update-silent/manifest#copy-silent",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/update-silent/manifest#copy-to-default-silent",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/update-silent/manifest#create-silent",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/update-silent/manifest#drop-default-silent",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/update-silent/manifest#drop-silent",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/update-silent/manifest#load-into-silent",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/update-silent/manifest#load-silent",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/update-silent/manifest#move-silent",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/update-silent/manifest#move-to-default-silent",
    // parser rejects valid input (27)
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_1",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_10",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_11",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_12",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_13",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_14",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_15",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_16",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_17",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_18",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_19",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_2",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_20",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_21",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_22",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_3",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_36",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_37",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_38",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_39",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_4",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_40",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_5",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_6",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_7",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_8",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_9",
    // parser accepts invalid input (missing validation) (12)
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/delete-insert/manifest#dawg-delete-insert-03",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/delete-insert/manifest#dawg-delete-insert-03b",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/delete-insert/manifest#dawg-delete-insert-05",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/delete-insert/manifest#dawg-delete-insert-06",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/delete-insert/manifest#dawg-delete-insert-07",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/delete-insert/manifest#dawg-delete-insert-07b",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/delete-insert/manifest#dawg-delete-insert-08",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/delete-insert/manifest#dawg-delete-insert-09",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_50",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_51",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_52",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/syntax-update-1/manifest#test_54",
    // update eval: resulting graph-store state differs (7)
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/basic-update/manifest#insert-05a",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/basic-update/manifest#insert-data-same-bnode",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/basic-update/manifest#insert-where-same-bnode",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/basic-update/manifest#insert-where-same-bnode2",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/delete-insert/manifest#dawg-delete-insert-01c",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/delete/manifest#dawg-delete-using-02a",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/delete/manifest#dawg-delete-using-06a",
    // engine: GRAPH blocks in DELETE WHERE unsupported (3)
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/delete-where/manifest#dawg-delete-where-02",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/delete-where/manifest#dawg-delete-where-04",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/delete-where/manifest#dawg-delete-where-06",
];

// All four block on one Turtle lexer bug: a blank-node label
// immediately followed by '.' (`_:o6.`) fails to lex — audit §4.2.5.
pub const SPARQL11_JSON_RES: &[&str] = &[
    // data load: Turtle/TriG parse error (4)
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/json-res/manifest#jsonres01",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/json-res/manifest#jsonres02",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/json-res/manifest#jsonres03",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/json-res/manifest#jsonres04",
];

// csv03 expects canonical xsd:double lexical form (1.0E6) in CSV output.
pub const SPARQL11_CSV_TSV: &[&str] = &[
    // result mismatch (1)
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/csv-tsv-res/manifest#csv03",
];

// Requires RDFS/OWL/RIF entailment regimes — a deliberate non-goal
// (audit §4.4). The 20 simple-entailment-answerable tests that pass
// today stay enforced; revisit this register if reasoning support lands.
pub const SPARQL11_ENTAILMENT: &[&str] = &[
    // result mismatch (44)
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#bind07",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#lang",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#paper-sparqldl-Q1",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#paper-sparqldl-Q1-rdfs",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#paper-sparqldl-Q2",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#paper-sparqldl-Q3",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#paper-sparqldl-Q4",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#parent10",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#parent3",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#parent4",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#parent5",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#parent6",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#parent7",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#parent8",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#parent9",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#plainLit",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#rdf01",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#rdfs01",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#rdfs02",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#rdfs03",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#rdfs04",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#rdfs05",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#rdfs06",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#rdfs07",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#rdfs09",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#rdfs10",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#rdfs11",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#rif01",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#rif03",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#rif04",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#rif06",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#simple1",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#simple2",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#simple3",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#simple4",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#simple5",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#simple6",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#simple7",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#simple8",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#sparqldl-02",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#sparqldl-10",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#sparqldl-11",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#sparqldl-12",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#sparqldl-13",
    // query execution error (4)
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#sparqldl-06",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#sparqldl-07",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#sparqldl-08",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#sparqldl-09",
    // data load: Turtle/TriG parse error (2)
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#owlds02",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/entailment/manifest#sparqldl-03",
];

pub const SPARQL12_GROUPING: &[&str] = &[
    // result mismatch (1)
    "https://w3c.github.io/rdf-tests/sparql/sparql12/grouping/manifest#group01",
];

pub const SPARQL12_CODEPOINT_ESCAPES: &[&str] = &[
    // parser rejects valid input (5)
    "https://w3c.github.io/rdf-tests/sparql/sparql12/codepoint-escapes/manifest#codepoint-esc-01",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/codepoint-escapes/manifest#codepoint-esc-02",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/codepoint-escapes/manifest#codepoint-esc-06",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/codepoint-escapes/manifest#codepoint-esc-07",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/codepoint-escapes/manifest#codepoint-esc-08",
    // parser accepts invalid input (missing validation) (1)
    "https://w3c.github.io/rdf-tests/sparql/sparql12/codepoint-escapes/manifest#codepoint-esc-bad-03",
];

pub const SPARQL12_SYNTAX_TRIPLE_TERMS_NEGATIVE: &[&str] = &[
    // parser accepts invalid input (missing validation) (1)
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-negative/manifest#syntax-update-anonreifier-02",
];

// RDF-star / SPARQL 1.2 triple-term syntax (<<( )>> and related) is not
// yet in the parser — audit §4.3 / Phase D.
pub const SPARQL12_SYNTAX_TRIPLE_TERMS_POSITIVE: &[&str] = &[
    // parser rejects valid input (87)
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#annotation-anonreifier-03",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#annotation-anonreifier-04",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#annotation-anonreifier-06",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#annotation-anonreifier-07",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#annotation-anonreifier-multiple-01",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#annotation-anonreifier-multiple-02",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#annotation-anonreifier-multiple-03",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#annotation-anonreifier-multiple-04",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#annotation-reifier-03",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#annotation-reifier-04",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#annotation-reifier-06",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#annotation-reifier-07",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#annotation-reifier-multiple-02",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#annotation-reifier-multiple-03",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#annotation-reifier-multiple-04",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#annotation-reifier-multiple-05",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#annotation-reifier-multiple-07",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#annotation-reifier-multiple-08",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#annotation-reifier-multiple-09",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#annotation-reifier-multiple-10",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#basic-anonreifier-02",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#basic-anonreifier-04",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#basic-anonreifier-07",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#basic-anonreifier-08",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#basic-anonreifier-09",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#basic-anonreifier-10",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#basic-anonreifier-11",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#basic-anonreifier-12",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#basic-anonreifier-13",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#basic-reifier-01",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#basic-reifier-02",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#basic-reifier-03",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#basic-reifier-04",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#basic-reifier-06",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#basic-reifier-07",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#basic-reifier-08",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#basic-reifier-09",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#basic-reifier-10",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#basic-reifier-11",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#basic-reifier-12",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#basic-reifier-13",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#basic-tripleterm-01",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#basic-tripleterm-02",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#basic-tripleterm-03",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#basic-tripleterm-04",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#basic-tripleterm-05",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#basic-tripleterm-06",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#basic-tripleterm-07",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#bnode-reifier-01",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#bnode-reifier-02",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#bnode-reifier-03",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#bnode-tripleterm-01",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#bnode-tripleterm-02",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#bnode-tripleterm-03",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#compound-all",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#compound-anonreifier",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#compound-reifier",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#compound-tripleterm",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#compound-tripleterm-subject",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#expr-tripleterm-01",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#expr-tripleterm-03",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#expr-tripleterm-04",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#expr-tripleterm-05",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#expr-tripleterm-06",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#inside-anonreifier-01",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#inside-anonreifier-02",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#inside-reifier-01",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#inside-reifier-02",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#inside-tripleterm-01",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#inside-tripleterm-02",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#nested-anonreifier-02",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#nested-reifier-01",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#nested-reifier-02",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#nested-tripleterm-01",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#nested-tripleterm-02",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#subject-tripleterm",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#update-anonreifier-04",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#update-anonreifier-05",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#update-reifier-01",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#update-reifier-03",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#update-reifier-04",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#update-reifier-05",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#update-reifier-07",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#update-tripleterm-01",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#update-tripleterm-03",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#update-tripleterm-04",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax-triple-terms-positive/manifest#update-tripleterm-05",
];

// Blocked on Turtle-star data loading and engine triple-term support —
// audit §4.3 / Phase D.
pub const SPARQL12_EVAL_TRIPLE_TERMS: &[&str] = &[
    // data load: Turtle/TriG parse error (40)
    "https://w3c.github.io/rdf-tests/sparql/sparql12/eval-triple-terms/manifest#basic-2",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/eval-triple-terms/manifest#basic-3",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/eval-triple-terms/manifest#basic-4",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/eval-triple-terms/manifest#basic-5",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/eval-triple-terms/manifest#basic-6",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/eval-triple-terms/manifest#basic-7",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/eval-triple-terms/manifest#basic-8",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/eval-triple-terms/manifest#basic-9",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/eval-triple-terms/manifest#construct-1",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/eval-triple-terms/manifest#construct-2",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/eval-triple-terms/manifest#construct-3",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/eval-triple-terms/manifest#construct-4",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/eval-triple-terms/manifest#construct-5",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/eval-triple-terms/manifest#expr-1",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/eval-triple-terms/manifest#graphs-1",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/eval-triple-terms/manifest#graphs-2",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/eval-triple-terms/manifest#op-1",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/eval-triple-terms/manifest#op-2",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/eval-triple-terms/manifest#order-1",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/eval-triple-terms/manifest#order-2",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/eval-triple-terms/manifest#pattern-1",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/eval-triple-terms/manifest#pattern-10",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/eval-triple-terms/manifest#pattern-11",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/eval-triple-terms/manifest#pattern-2",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/eval-triple-terms/manifest#pattern-3",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/eval-triple-terms/manifest#pattern-3-nomatch",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/eval-triple-terms/manifest#pattern-4",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/eval-triple-terms/manifest#pattern-5",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/eval-triple-terms/manifest#pattern-6",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/eval-triple-terms/manifest#pattern-7",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/eval-triple-terms/manifest#pattern-8",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/eval-triple-terms/manifest#pattern-8-nomatch",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/eval-triple-terms/manifest#pattern-9",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/eval-triple-terms/manifest#results-reifiedtriples-1j",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/eval-triple-terms/manifest#results-reifiedtriples-1x",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/eval-triple-terms/manifest#results-tripleterms-1j",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/eval-triple-terms/manifest#results-tripleterms-1x",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/eval-triple-terms/manifest#update-1",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/eval-triple-terms/manifest#update-2",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/eval-triple-terms/manifest#update-3",
    // query execution error (1)
    "https://w3c.github.io/rdf-tests/sparql/sparql12/eval-triple-terms/manifest#expr-2",
];

pub const SPARQL12_EXPRESSION: &[&str] = &[
    // result mismatch (1)
    "https://w3c.github.io/rdf-tests/sparql/sparql12/expression/manifest#not-not",
];

// Base-direction language literals (`@en--ltr`) — audit §4.3.
pub const SPARQL12_LANG_BASEDIR: &[&str] = &[
    // query execution error (8)
    "https://w3c.github.io/rdf-tests/sparql/sparql12/lang-basedir/manifest#concat",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/lang-basedir/manifest#contains",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/lang-basedir/manifest#datatype",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/lang-basedir/manifest#haslang",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/lang-basedir/manifest#haslangdir",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/lang-basedir/manifest#langdir",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/lang-basedir/manifest#langdir-literal",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/lang-basedir/manifest#strlangdir",
    // result mismatch (2)
    "https://w3c.github.io/rdf-tests/sparql/sparql12/lang-basedir/manifest#lang",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/lang-basedir/manifest#strlang",
];

pub const SPARQL12_RDF11: &[&str] = &[
    // query execution error (2)
    "https://w3c.github.io/rdf-tests/sparql/sparql12/rdf11/manifest#langstring-datatype",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/rdf11/manifest#plain-string-datatype",
    // result mismatch (1)
    "https://w3c.github.io/rdf-tests/sparql/sparql12/rdf11/manifest#plain-string-same",
];

pub const SPARQL12_SYNTAX: &[&str] = &[
    // parser accepts invalid input (missing validation) (2)
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax/manifest#duplicated-values-variable",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/syntax/manifest#nested-aggregate-functions",
];

// VERSION declaration support — audit §4.3.
pub const SPARQL12_VERSION: &[&str] = &[
    // parser rejects valid input (3)
    "https://w3c.github.io/rdf-tests/sparql/sparql12/version/manifest#version-01",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/version/manifest#version-02",
    "https://w3c.github.io/rdf-tests/sparql/sparql12/version/manifest#version-05",
];

// Path cardinality / duplicate semantics — sequence-path results differ in
// multiplicity (`p1/p2` path counting vs `*`/`+` distinct nodes) — plus the
// zero-variable `SELECT *` projection over a both-bound path (pp36).
// Audit §4.2.7 (hot-operator semantics; fix must preserve `*`/`+` fast path).
pub const SPARQL11_PROPERTY_PATH: &[&str] = &[
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/property-path/manifest#pp16",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/property-path/manifest#pp34",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/property-path/manifest#pp35",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/property-path/manifest#pp36",
];

// SERVICE evaluation requires live external SPARQL endpoints, which a unit
// test environment cannot provide. Revisit with an in-process mock endpoint
// when federation execution lands (audit §4.4).
pub const SPARQL11_SERVICE: &[&str] = &[
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/service/manifest#service1",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/service/manifest#service2",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/service/manifest#service3",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/service/manifest#service4a",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/service/manifest#service5",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/service/manifest#service6",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/service/manifest#service7",
];

// SPARQL Protocol conformance requires an HTTP client/server harness —
// not applicable to database-engine unit testing (audit §4.4).
pub const SPARQL11_PROTOCOL: &[&str] = &[
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/protocol/manifest#bad_multiple_queries",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/protocol/manifest#bad_multiple_updates",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/protocol/manifest#bad_query_method",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/protocol/manifest#bad_query_missing_direct_type",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/protocol/manifest#bad_query_missing_form_type",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/protocol/manifest#bad_query_non_utf8",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/protocol/manifest#bad_query_syntax",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/protocol/manifest#bad_query_wrong_media_type",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/protocol/manifest#bad_update_dataset_conflict",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/protocol/manifest#bad_update_get",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/protocol/manifest#bad_update_missing_form_type",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/protocol/manifest#bad_update_non_utf8",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/protocol/manifest#bad_update_syntax",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/protocol/manifest#bad_update_wrong_media_type",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/protocol/manifest#query_content_type_ask",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/protocol/manifest#query_content_type_construct",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/protocol/manifest#query_content_type_describe",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/protocol/manifest#query_content_type_select",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/protocol/manifest#query_dataset_default_graphs_get",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/protocol/manifest#query_dataset_default_graphs_post",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/protocol/manifest#query_dataset_full",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/protocol/manifest#query_dataset_named_graphs_get",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/protocol/manifest#query_dataset_named_graphs_post",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/protocol/manifest#query_get",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/protocol/manifest#query_multiple_dataset",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/protocol/manifest#query_post_direct",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/protocol/manifest#query_post_form",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/protocol/manifest#update_base_uri",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/protocol/manifest#update_dataset_default_graph",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/protocol/manifest#update_dataset_default_graphs",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/protocol/manifest#update_dataset_full",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/protocol/manifest#update_dataset_named_graphs",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/protocol/manifest#update_post_direct",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/protocol/manifest#update_post_form",
];

// Requires a running SPARQL server to introspect (audit §4.4).
pub const SPARQL11_SERVICE_DESCRIPTION: &[&str] = &[
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/service-description/manifest#conforms-to-schema",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/service-description/manifest#has-endpoint-triple",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/service-description/manifest#returns-rdf",
];

// Graph Store Protocol requires HTTP endpoint infrastructure (audit §4.4).
pub const SPARQL11_HTTP_RDF_UPDATE: &[&str] = &[
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/http-rdf-update/manifest#delete__existing_graph",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/http-rdf-update/manifest#delete__nonexistent_graph",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/http-rdf-update/manifest#get_of_delete__existing_graph",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/http-rdf-update/manifest#get_of_post__after_noop",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/http-rdf-update/manifest#get_of_post__create__new_graph",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/http-rdf-update/manifest#get_of_post__existing_graph",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/http-rdf-update/manifest#get_of_post__multipart_formdata",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/http-rdf-update/manifest#get_of_put__default_graph",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/http-rdf-update/manifest#get_of_put__graph_already_in_store",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/http-rdf-update/manifest#get_of_put__initial_state",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/http-rdf-update/manifest#head_on_a_nonexisting_graph",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/http-rdf-update/manifest#head_on_an_existing_graph",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/http-rdf-update/manifest#post__create__new_graph",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/http-rdf-update/manifest#post__existing_graph",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/http-rdf-update/manifest#post__multipart_formdata",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/http-rdf-update/manifest#put__default_graph",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/http-rdf-update/manifest#put__graph_already_in_store",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/http-rdf-update/manifest#put__initial_state",
    "http://www.w3.org/2009/sparql/docs/tests/data-sparql11/http-rdf-update/manifest#put__mismatched_payload",
];
