//! Canonicalization + SHA-256 of a SPARQL-results-JSON document.
//!
//! Both the native ledger and the R2RML/Iceberg virtual engine are asked to
//! render results as SPARQL JSON (`FormatterConfig::sparql_json()`), so a single
//! canonicalizer serves both sides. The result is an **order-independent
//! multiset hash**: rows are canonicalized cell-by-cell, then the whole row-set
//! is sorted before hashing, so two engines that emit the same bindings in a
//! different order still hash equal.
//!
//! Cell canonicalization keys off the RDF term kind and (for literals) the
//! datatype:
//! - IRIs / bnodes: the IRI verbatim (bnodes collapse to a constant — they are
//!   not stable across engines and none of the seed queries project them).
//! - integer-family literals: reparsed and re-emitted (strips leading zeros /
//!   whitespace).
//! - decimal literals: shortest round-trip of the parsed value (so `100` and
//!   `100.0` collapse).
//! - float/double literals: quantized to 12 significant digits.
//! - everything else (string, date, dateTime, boolean, ...): the lexical form
//!   verbatim.

use sha2::{Digest, Sha256};
use serde_json::Value;

/// Unit separator between cells of a row (cannot appear in canonical cells).
const CELL_SEP: char = '\u{1f}';
/// Record separator between rows.
const ROW_SEP: char = '\u{1e}';
/// Sentinel for an unbound variable in a row.
const UNBOUND: &str = "\u{0}UNBOUND";

/// Canonicalized result: row count, hex hash, and the sorted canonical rows.
pub struct Canonical {
    pub rows: usize,
    pub hash: String,
    /// Sorted canonical row strings (cells joined by the unit separator).
    pub canonical_rows: Vec<String>,
}

impl Canonical {
    /// The first `n` canonical rows (for `--keep-heads`).
    pub fn heads(&self, n: usize) -> Vec<String> {
        self.canonical_rows.iter().take(n).cloned().collect()
    }
}

/// Canonicalize a SPARQL-results-JSON document (SELECT or ASK).
pub fn canonicalize_sparql_json(doc: &Value) -> Canonical {
    // ASK: a single boolean.
    if let Some(b) = doc.get("boolean").and_then(Value::as_bool) {
        let row = format!("BOOLEAN{CELL_SEP}{b}");
        return finish(vec![row]);
    }

    // SELECT: head.vars drives a stable column order; results.bindings are rows.
    let mut vars: Vec<String> = doc
        .get("head")
        .and_then(|h| h.get("vars"))
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    vars.sort();

    let bindings = doc
        .get("results")
        .and_then(|r| r.get("bindings"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut rows = Vec::with_capacity(bindings.len());
    for binding in &bindings {
        let mut cells = Vec::with_capacity(vars.len());
        for var in &vars {
            cells.push(canonical_cell(binding.get(var)));
        }
        rows.push(cells.join(&CELL_SEP.to_string()));
    }
    finish(rows)
}

/// Sort, join, and hash a set of canonical row strings.
fn finish(mut rows: Vec<String>) -> Canonical {
    rows.sort();
    let mut hasher = Sha256::new();
    for row in &rows {
        hasher.update(row.as_bytes());
        hasher.update([ROW_SEP as u8]);
    }
    let hash = hex::encode(hasher.finalize());
    Canonical {
        rows: rows.len(),
        hash,
        canonical_rows: rows,
    }
}

/// Canonical string for one SPARQL-JSON binding cell (or an unbound variable).
fn canonical_cell(cell: Option<&Value>) -> String {
    let Some(cell) = cell else {
        return UNBOUND.to_string();
    };
    let kind = cell.get("type").and_then(Value::as_str).unwrap_or("");
    let value = cell.get("value").and_then(Value::as_str).unwrap_or("");
    match kind {
        "uri" => format!("<{value}>"),
        "bnode" => "_:b".to_string(),
        _ => {
            let datatype = cell.get("datatype").and_then(Value::as_str);
            canonical_literal(value, datatype)
        }
    }
}

/// Canonical lexical for a typed/untyped literal.
fn canonical_literal(value: &str, datatype: Option<&str>) -> String {
    let Some(dt) = datatype else {
        return value.to_string();
    };
    let local = dt.rsplit(['#', '/']).next().unwrap_or(dt);
    match local {
        "integer" | "int" | "long" | "short" | "byte" | "nonNegativeInteger"
        | "nonPositiveInteger" | "negativeInteger" | "positiveInteger"
        | "unsignedLong" | "unsignedInt" | "unsignedShort" | "unsignedByte" => {
            match value.trim().parse::<i128>() {
                Ok(n) => n.to_string(),
                Err(_) => value.to_string(),
            }
        }
        "decimal" => match value.trim().parse::<f64>() {
            // Shortest round-trip collapses `100` / `100.0` / `100.00`.
            Ok(f) => format!("{f}"),
            Err(_) => value.to_string(),
        },
        "double" | "float" => match value.trim().parse::<f64>() {
            Ok(f) => quantize(f),
            Err(_) => value.to_string(),
        },
        _ => value.to_string(),
    }
}

/// Quantize a float to 12 significant digits (canonical scientific form).
fn quantize(f: f64) -> String {
    if !f.is_finite() {
        return format!("{f}");
    }
    if f == 0.0 {
        return "0".to_string();
    }
    // 11 fractional digits in scientific form == 12 significant digits.
    format!("{f:.11e}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sparql_doc(vars: &[&str], rows: Vec<Value>) -> Value {
        json!({
            "head": { "vars": vars },
            "results": { "bindings": rows }
        })
    }

    #[test]
    fn identical_bindings_hash_equal_regardless_of_row_order() {
        let a = sparql_doc(
            &["s", "n"],
            vec![
                json!({"s": {"type":"uri","value":"http://x/1"}, "n": {"type":"literal","value":"5","datatype":"http://www.w3.org/2001/XMLSchema#integer"}}),
                json!({"s": {"type":"uri","value":"http://x/2"}, "n": {"type":"literal","value":"6","datatype":"http://www.w3.org/2001/XMLSchema#integer"}}),
            ],
        );
        let b = sparql_doc(
            &["s", "n"],
            vec![
                json!({"s": {"type":"uri","value":"http://x/2"}, "n": {"type":"literal","value":"6","datatype":"http://www.w3.org/2001/XMLSchema#integer"}}),
                json!({"s": {"type":"uri","value":"http://x/1"}, "n": {"type":"literal","value":"5","datatype":"http://www.w3.org/2001/XMLSchema#integer"}}),
            ],
        );
        let ca = canonicalize_sparql_json(&a);
        let cb = canonicalize_sparql_json(&b);
        assert_eq!(ca.rows, 2);
        assert_eq!(ca.hash, cb.hash, "row order must not affect the hash");
    }

    #[test]
    fn integer_and_decimal_lexical_variants_collapse() {
        // `100` (integer) rendered by one engine, `100.0` (decimal) by another.
        let a = sparql_doc(
            &["v"],
            vec![json!({"v": {"type":"literal","value":"0100","datatype":"http://www.w3.org/2001/XMLSchema#integer"}})],
        );
        let b = sparql_doc(
            &["v"],
            vec![json!({"v": {"type":"literal","value":"100","datatype":"http://www.w3.org/2001/XMLSchema#integer"}})],
        );
        assert_eq!(
            canonicalize_sparql_json(&a).hash,
            canonicalize_sparql_json(&b).hash
        );

        let d1 = sparql_doc(
            &["v"],
            vec![json!({"v": {"type":"literal","value":"100.0","datatype":"http://www.w3.org/2001/XMLSchema#decimal"}})],
        );
        let d2 = sparql_doc(
            &["v"],
            vec![json!({"v": {"type":"literal","value":"100.00","datatype":"http://www.w3.org/2001/XMLSchema#decimal"}})],
        );
        assert_eq!(
            canonicalize_sparql_json(&d1).hash,
            canonicalize_sparql_json(&d2).hash
        );
    }

    #[test]
    fn floats_within_12_sig_digits_hash_equal() {
        let a = sparql_doc(
            &["v"],
            vec![json!({"v": {"type":"literal","value":"3.14159265358979","datatype":"http://www.w3.org/2001/XMLSchema#double"}})],
        );
        // Differs only past the 12th significant digit.
        let b = sparql_doc(
            &["v"],
            vec![json!({"v": {"type":"literal","value":"3.141592653589792","datatype":"http://www.w3.org/2001/XMLSchema#double"}})],
        );
        assert_eq!(
            canonicalize_sparql_json(&a).hash,
            canonicalize_sparql_json(&b).hash
        );
    }

    #[test]
    fn unbound_variable_is_stable_and_distinct() {
        let bound = sparql_doc(
            &["a", "b"],
            vec![json!({"a": {"type":"uri","value":"http://x/1"}, "b": {"type":"literal","value":"x"}})],
        );
        let unbound = sparql_doc(
            &["a", "b"],
            vec![json!({"a": {"type":"uri","value":"http://x/1"}})],
        );
        assert_ne!(
            canonicalize_sparql_json(&bound).hash,
            canonicalize_sparql_json(&unbound).hash
        );
    }

    #[test]
    fn ask_boolean_canonicalizes() {
        let t = json!({"head": {}, "boolean": true});
        let f = json!({"head": {}, "boolean": false});
        assert_eq!(canonicalize_sparql_json(&t).rows, 1);
        assert_ne!(
            canonicalize_sparql_json(&t).hash,
            canonicalize_sparql_json(&f).hash
        );
    }
}
