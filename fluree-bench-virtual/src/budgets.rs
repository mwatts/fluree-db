//! Perf regression budgets for `vbench compare --gate`.
//!
//! Modeled on `fluree-bench-support::budget::RegressionBudget` (a
//! `default_budget_pct` + explicit overrides), but keyed by **query id** with
//! **tag-level defaults** split by gating class: native queries get a tighter
//! budget than virtual (which carries live-Snowflake variance). Cold runs are
//! advisory-only and never gate. `budgets.json` lives at the crate root.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// The `budgets.json` document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Budgets {
    #[serde(default = "one")]
    pub schema_version: u32,
    /// Tag-level default budgets by gating class.
    pub default_budget_pct: DefaultBudgets,
    /// Documents that cold runs are advisory-only (they never gate regardless).
    #[serde(default)]
    pub cold: String,
    /// Per-query budget overrides (query id → percent) — win over the defaults.
    #[serde(default)]
    pub overrides: BTreeMap<String, f64>,
}

/// Tag-level defaults, split by gating class.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefaultBudgets {
    /// Allowed % slowdown for a native hot wall.
    pub native: f64,
    /// Allowed % slowdown for a virtual hot wall.
    pub virtual_hot: f64,
}

fn one() -> u32 {
    1
}

impl Default for Budgets {
    fn default() -> Self {
        Self {
            schema_version: 1,
            default_budget_pct: DefaultBudgets {
                native: 10.0,
                virtual_hot: 20.0,
            },
            cold: "advisory".to_string(),
            overrides: BTreeMap::new(),
        }
    }
}

impl Budgets {
    /// Load `budgets.json`, or fall back to the built-in defaults if absent.
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("reading budgets {}", path.display()))?;
        serde_json::from_str(&raw).with_context(|| format!("parsing budgets {}", path.display()))
    }

    /// The budget percent that gates a query, or `None` when the record is
    /// advisory (cold) and must never gate. A per-query override wins over the
    /// class default.
    pub fn budget_pct(&self, query_id: &str, is_virtual: bool, cold: bool) -> Option<f64> {
        if cold {
            return None;
        }
        if let Some(pct) = self.overrides.get(query_id) {
            return Some(*pct);
        }
        Some(if is_virtual {
            self.default_budget_pct.virtual_hot
        } else {
            self.default_budget_pct.native
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_budgets_parse() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("budgets.json");
        let b = Budgets::load(&path).expect("budgets.json must parse");
        assert_eq!(b.default_budget_pct.native, 10.0);
        assert_eq!(b.default_budget_pct.virtual_hot, 20.0);
    }

    #[test]
    fn budget_lookup_class_and_override_and_cold() {
        let mut b = Budgets::default();
        assert_eq!(b.budget_pct("q001", false, false), Some(10.0));
        assert_eq!(b.budget_pct("q001", true, false), Some(20.0));
        // Cold is advisory — never gates.
        assert_eq!(b.budget_pct("q001", true, true), None);
        // Override wins.
        b.overrides.insert("q015".to_string(), 50.0);
        assert_eq!(b.budget_pct("q015", true, false), Some(50.0));
    }
}
