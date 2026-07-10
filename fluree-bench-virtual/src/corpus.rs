//! Corpus manifest: the tagged query catalog and its validation.
//!
//! `corpus/manifest.json` lists every query with its file, tags, tables, BI
//! question, expected-row bound, and subset membership. [`Corpus::load`]
//! validates the manifest before any run: ids are unique, every `.rq` file
//! exists, tags are drawn from the closed [`Tag`] enum, and the `smoke` subset
//! covers every tag that appears anywhere in the corpus (so a smoke run
//! exercises the full pathway surface).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Closed set of pathway tags. Adding a query pathway means adding a variant
/// here — an unknown tag in the manifest is a load error, not a silent pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tag {
    BgpStar,
    Join,
    FilterRange,
    OrderBy,
    GroupBy,
    Aggregate,
    Count,
}

/// Whether row-order carries meaning for a query (metadata only — the result
/// hash is always an order-independent multiset).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderSensitivity {
    None,
    ByKeys,
}

/// Expected row count: an exact value or an inclusive `[min, max]` range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ExpectedRows {
    Exact(usize),
    Range([usize; 2]),
}

impl ExpectedRows {
    /// Whether `n` satisfies this bound.
    pub fn contains(&self, n: usize) -> bool {
        match self {
            Self::Exact(x) => n == *x,
            Self::Range([lo, hi]) => n >= *lo && n <= *hi,
        }
    }
}

impl std::fmt::Display for ExpectedRows {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Exact(x) => write!(f, "{x}"),
            Self::Range([lo, hi]) => write!(f, "[{lo},{hi}]"),
        }
    }
}

fn default_timeout_s() -> u64 {
    120
}

/// One catalogued query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryDef {
    pub id: String,
    /// Path to the `.rq` file, relative to the corpus directory.
    pub file: PathBuf,
    pub bi_question: String,
    pub tags: Vec<Tag>,
    pub tables: Vec<String>,
    pub class: String,
    pub expected_rows: ExpectedRows,
    pub order_sensitive: OrderSensitivity,
    #[serde(default = "default_timeout_s")]
    pub timeout_s: u64,
    pub subsets: Vec<String>,
}

impl QueryDef {
    pub fn in_subset(&self, subset: &str) -> bool {
        self.subsets.iter().any(|s| s == subset)
    }
}

#[derive(Debug, Clone, Deserialize)]
struct Manifest {
    #[allow(dead_code)]
    corpus_version: u32,
    #[allow(dead_code)]
    #[serde(default)]
    description: String,
    queries: Vec<QueryDef>,
}

/// The validated corpus plus the directory its query files live in.
pub struct Corpus {
    pub dir: PathBuf,
    pub queries: Vec<QueryDef>,
}

impl Corpus {
    /// Load and validate `<dir>/manifest.json`.
    pub fn load(dir: &Path) -> Result<Self> {
        let manifest_path = dir.join("manifest.json");
        let raw = std::fs::read_to_string(&manifest_path)
            .with_context(|| format!("reading manifest {}", manifest_path.display()))?;
        let manifest: Manifest = serde_json::from_str(&raw)
            .with_context(|| format!("parsing manifest {}", manifest_path.display()))?;
        let corpus = Self {
            dir: dir.to_path_buf(),
            queries: manifest.queries,
        };
        corpus.validate()?;
        Ok(corpus)
    }

    /// Read the SPARQL text for a query.
    pub fn read_query(&self, def: &QueryDef) -> Result<String> {
        let path = self.dir.join(&def.file);
        std::fs::read_to_string(&path)
            .with_context(|| format!("reading query {}", path.display()))
    }

    /// Queries in a subset (or all queries when `subset` is `None`).
    pub fn select(&self, subset: Option<&str>) -> Vec<&QueryDef> {
        self.queries
            .iter()
            .filter(|q| subset.is_none_or(|s| q.in_subset(s)))
            .collect()
    }

    /// Look up a single query by id.
    pub fn get(&self, id: &str) -> Option<&QueryDef> {
        self.queries.iter().find(|q| q.id == id)
    }

    fn validate(&self) -> Result<()> {
        // Unique ids.
        let mut seen = BTreeSet::new();
        for q in &self.queries {
            if !seen.insert(q.id.as_str()) {
                anyhow::bail!("duplicate query id '{}' in manifest", q.id);
            }
        }

        // Every query file exists and is non-empty.
        for q in &self.queries {
            let path = self.dir.join(&q.file);
            let text = std::fs::read_to_string(&path).with_context(|| {
                format!("query '{}' references missing file {}", q.id, path.display())
            })?;
            if text.trim().is_empty() {
                anyhow::bail!("query '{}' file {} is empty", q.id, path.display());
            }
        }

        // Smoke covers every tag that appears anywhere in the corpus.
        let all_tags: BTreeSet<Tag> = self.queries.iter().flat_map(|q| q.tags.iter().copied()).collect();
        let smoke_tags: BTreeSet<Tag> = self
            .queries
            .iter()
            .filter(|q| q.in_subset("smoke"))
            .flat_map(|q| q.tags.iter().copied())
            .collect();
        let uncovered: Vec<Tag> = all_tags.difference(&smoke_tags).copied().collect();
        if !uncovered.is_empty() {
            anyhow::bail!(
                "smoke subset does not cover every tag present in the corpus; uncovered: {:?}",
                uncovered
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped corpus is valid (unique ids, files present, smoke covers all
    /// tags). This is the corpus-validation unit test the quality bar requires.
    #[test]
    fn shipped_corpus_is_valid() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus");
        let corpus = Corpus::load(&dir).expect("shipped corpus must validate");
        assert_eq!(corpus.queries.len(), 5, "seed corpus has five queries");
        // Every seed query is in the smoke subset.
        assert_eq!(corpus.select(Some("smoke")).len(), 5);
    }

    #[test]
    fn expected_rows_bounds() {
        assert!(ExpectedRows::Exact(1).contains(1));
        assert!(!ExpectedRows::Exact(1).contains(2));
        assert!(ExpectedRows::Range([1, 10]).contains(1));
        assert!(ExpectedRows::Range([1, 10]).contains(10));
        assert!(!ExpectedRows::Range([1, 10]).contains(11));
    }

    #[test]
    fn duplicate_ids_rejected() {
        let q = QueryDef {
            id: "dup".to_string(),
            file: PathBuf::from("queries/q001_count_class.rq"),
            bi_question: String::new(),
            tags: vec![Tag::Count],
            tables: vec![],
            class: "dims-only".to_string(),
            expected_rows: ExpectedRows::Exact(1),
            order_sensitive: OrderSensitivity::None,
            timeout_s: 120,
            subsets: vec!["smoke".to_string()],
        };
        let corpus = Corpus {
            dir: Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus"),
            queries: vec![q.clone(), q],
        };
        assert!(corpus.validate().is_err(), "duplicate ids must be rejected");
    }
}
