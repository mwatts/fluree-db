//! The virtual-pathway span allowlist and its aggregation into [`Counters`].
//!
//! Every name here was verified against a `debug_span!`/`instrument` callsite in
//! the engine (file:line cited below); they are stable string literals, so the
//! allowlist is safe to pin as the schema of the pathway counters:
//!
//! | span | site | numeric fields |
//! |---|---|---|
//! | `r2rml.scan_table`   | `fluree-db-api/src/graph_source/r2rml.rs` (scan setup: loadTable + planning) | `projection_len` |
//! | `r2rml.load_table`   | `fluree-db-api/src/graph_source/r2rml.rs` (cold REST/OAuth catalog load) | — |
//! | `iceberg.scan_plan`  | `fluree-db-iceberg/src/scan/send_planner.rs` (manifest read + file pruning) | `files_selected`, `files_pruned`, `estimated_row_count` |
//! | `iceberg.parquet_read` | `fluree-db-api/src/graph_source/r2rml.rs` (per-file decode, in spawned tasks) | `file_size` |
//! | `iceberg.oauth_token`  | `fluree-db-iceberg/src/auth/oauth2.rs` (OAuth token mint) | — |
//!
//! Engine-stage spans in `fluree-db-query` (`operator_open`, `reasoning_prep`)
//! were considered but left out: they are generic to every query (native and
//! virtual alike) and add no native-vs-virtual signal, and there is no stable
//! `query_run`/`query_plan` span literal to cite. Keeping the allowlist to the
//! five virtual-only spans keeps the counter schema minimal and stable.

use fluree_bench_support::tracing::SpanRecord;

use crate::schema::{Counters, SpanAgg};

/// Spans captured for the pathway counters. Passed to `BenchSpanCapture::layer`
/// and `BenchSpanLayer::filter` so nothing else is captured.
pub const SPAN_ALLOWLIST: &[&str] = &[
    "r2rml.scan_table",
    "r2rml.load_table",
    "iceberg.parquet_read",
    "iceberg.scan_plan",
    "iceberg.oauth_token",
];

/// Spans that MUST fire on any non-trivial virtual scan. A virtual execution
/// where none of these appear either didn't hit the R2RML engine or ran with
/// tracing mis-installed — that's what `spans_missing` flags.
///
/// Deliberately excludes the data-/cache-dependent spans:
/// - `iceberg.parquet_read` — a metadata-only COUNT can answer from row-count
///   stats without reading any Parquet.
/// - `r2rml.load_table` / `iceberg.oauth_token` — fire only on a cold catalog /
///   OAuth miss; a warm cross-query cache skips them.
pub const EXPECTED_FOR_VIRTUAL: &[&str] = &["r2rml.scan_table", "iceberg.scan_plan"];

/// Fold a rep's captured spans into per-span timing aggregates plus the summed
/// numeric fields the Iceberg planner/reader record.
pub fn aggregate(records: &[SpanRecord]) -> Counters {
    let mut counters = Counters::default();
    for record in records {
        if !SPAN_ALLOWLIST.contains(&record.name) {
            continue;
        }
        let agg = counters
            .spans
            .entry(record.name.to_string())
            .or_insert_with(SpanAgg::default);
        agg.n += 1;
        agg.total_us += record.elapsed_us;
        agg.max_us = agg.max_us.max(record.elapsed_us);

        counters.files_selected += field_u64(record, "files_selected");
        counters.files_pruned += field_u64(record, "files_pruned");
        counters.estimated_row_count += field_u64(record, "estimated_row_count");
        counters.file_size += field_u64(record, "file_size");
    }
    counters
}

/// Read a numeric span field as a non-negative `u64` (0 if absent/negative).
fn field_u64(record: &SpanRecord, key: &str) -> u64 {
    record
        .fields
        .get(key)
        .and_then(|v| v.as_i64().or_else(|| v.as_u64().map(|u| u as i64)))
        .map(|n| u64::try_from(n).unwrap_or(0))
        .unwrap_or(0)
}

/// Expected-for-virtual spans that did not fire. Empty for native targets.
pub fn spans_missing(counters: &Counters, is_virtual: bool) -> Vec<String> {
    if !is_virtual {
        return Vec::new();
    }
    EXPECTED_FOR_VIRTUAL
        .iter()
        .filter(|name| counters.spans.get(**name).map_or(true, |a| a.n == 0))
        .map(|name| (*name).to_string())
        .collect()
}

/// Convenience accessor: number of spans of `name` seen (0 if none).
pub fn span_count(counters: &Counters, name: &str) -> u64 {
    counters.spans.get(name).map_or(0, |a| a.n)
}

/// Convenience accessor: total microseconds spent in spans of `name`.
pub fn span_total_us(counters: &Counters, name: &str) -> u64 {
    counters.spans.get(name).map_or(0, |a| a.total_us)
}

/// Aggregation stats keyed on nothing but the sink layout — no runtime span
/// capture, so it's a plain fixture test.
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Map, Number, Value};

    fn rec(name: &'static str, elapsed_us: u64, fields: &[(&str, i64)]) -> SpanRecord {
        let mut map = Map::new();
        for (k, v) in fields {
            map.insert((*k).to_string(), Value::Number(Number::from(*v)));
        }
        SpanRecord {
            name,
            parent: None,
            elapsed_us,
            fields: map,
        }
    }

    #[test]
    fn aggregates_counts_timing_and_numeric_fields() {
        let records = vec![
            rec("r2rml.scan_table", 1000, &[]),
            rec(
                "iceberg.scan_plan",
                500,
                &[
                    ("files_selected", 3),
                    ("files_pruned", 7),
                    ("estimated_row_count", 1200),
                ],
            ),
            rec("iceberg.parquet_read", 800, &[("file_size", 4096)]),
            rec("iceberg.parquet_read", 900, &[("file_size", 8192)]),
            // Not on the allowlist — ignored.
            rec("some.other.span", 5, &[]),
        ];
        let c = aggregate(&records);
        assert_eq!(span_count(&c, "iceberg.parquet_read"), 2);
        assert_eq!(span_total_us(&c, "iceberg.parquet_read"), 1700);
        assert_eq!(c.spans.get("iceberg.parquet_read").unwrap().max_us, 900);
        assert_eq!(c.files_selected, 3);
        assert_eq!(c.files_pruned, 7);
        assert_eq!(c.estimated_row_count, 1200);
        assert_eq!(c.file_size, 12288);
        assert!(!c.spans.contains_key("some.other.span"));
    }

    #[test]
    fn spans_missing_flags_absent_expected_spans_for_virtual_only() {
        // Only scan_table fired; scan_plan is missing.
        let c = aggregate(&[rec("r2rml.scan_table", 10, &[])]);
        assert_eq!(spans_missing(&c, true), vec!["iceberg.scan_plan".to_string()]);
        // Native target: never flagged.
        assert!(spans_missing(&c, false).is_empty());
        // Both expected spans present: nothing missing.
        let full = aggregate(&[
            rec("r2rml.scan_table", 10, &[]),
            rec("iceberg.scan_plan", 10, &[]),
        ]);
        assert!(spans_missing(&full, true).is_empty());
    }
}
