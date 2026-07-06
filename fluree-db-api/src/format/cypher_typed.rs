//! Typed Cypher result cells for value-typed transports (Bolt).
//!
//! The JSON transport flattens everything to strings/numbers
//! ([`super::cypher`]); PackStream carries real graph and temporal values.
//! This module walks the same columns/rows as the JSON formatter but keeps
//! cells **typed**: node refs hydrate into [`CypherNode`] (labels +
//! properties fetched per subject at format time), relationship values keep
//! their endpoints and annotation properties, and temporal literals carry
//! epoch components instead of ISO strings. Naming (labels, property keys,
//! relationship types) reuses the engine's Cypher rule
//! ([`fluree_db_query::eval::cypher_name_from_iri`]) so `labels(n)` and a
//! returned node never disagree.
//!
//! Hydration reads raw SPOT state (snapshot + overlay at the view's `t`)
//! and therefore **must not run under a view policy** — the caller checks
//! `GraphDb::has_policy()` and errors rather than leaking filtered
//! properties.

use std::collections::HashMap;

use futures::future::BoxFuture;
use futures::FutureExt;
use serde_json::Value as JsonValue;

use super::iri::IriCompactor;
use super::{FormatError, Result};
use crate::query::QueryResult;
use crate::view::GraphDb;
use fluree_db_core::{FlakeValue, IndexType, RangeMatch, RangeOptions, RangeTest, Sid};
use fluree_db_query::binding::Binding;
use fluree_db_query::eval::cypher_name_from_iri;

/// Predicates under the Fluree system namespace (`db:reifies*`, the
/// `db:Node` existence marker's class triples, ...) are internal wiring,
/// never user properties.
const FLUREE_SYSTEM_NS: &str = "https://ns.flur.ee/";

/// One result cell, typed for transports with richer value models than JSON.
#[derive(Debug, Clone, PartialEq)]
pub enum CypherCell {
    /// Plain scalar/string/bool/@json — RDF-faithful JSON as produced by
    /// the shared per-binding formatter.
    Value(JsonValue),
    /// `xsd:decimal` exact lexical form (PackStream has no decimal type;
    /// the transport decides the degradation).
    Decimal(String),
    /// Arbitrary-precision integer that may exceed i64.
    BigInt(String),
    Temporal(CypherTemporal),
    List(Vec<CypherCell>),
    Map(Vec<(String, CypherCell)>),
    Node(Box<CypherNode>),
    Relationship(Box<CypherRelationship>),
    Path(Box<CypherPath>),
}

/// A hydrated node: durable identity (IRI), Cypher labels, and properties.
#[derive(Debug, Clone, PartialEq)]
pub struct CypherNode {
    /// Full IRI — the durable identity (`elementId` on Bolt).
    pub iri: String,
    /// `rdf:type` classes as Cypher label names (the `db:Node` existence
    /// marker is hidden, matching `labels()`).
    pub labels: Vec<String>,
    pub properties: Vec<(String, CypherCell)>,
}

/// A relationship value: endpoints, type, and annotation properties.
#[derive(Debug, Clone, PartialEq)]
pub struct CypherRelationship {
    pub start_iri: String,
    pub end_iri: String,
    /// Cypher relationship type (predicate IRI local name).
    pub type_name: String,
    /// The reifier subject's IRI when the edge is reified — the durable
    /// relationship identity where one exists.
    pub reifier_iri: Option<String>,
    pub properties: Vec<(String, CypherCell)>,
}

/// A path as Bolt models it: unique node and relationship lists plus the
/// alternating (relationship, node) index sequence describing the walk
/// from `nodes[0]`. Relationship indices are 1-based and negated when the
/// hop traverses the edge end→start; node indices are 0-based.
#[derive(Debug, Clone, PartialEq)]
pub struct CypherPath {
    pub nodes: Vec<CypherNode>,
    pub rels: Vec<CypherRelationship>,
    pub indices: Vec<i64>,
}

/// A temporal literal with epoch components. `iso` keeps the original
/// lexical form for transports (or clients) that prefer strings.
#[derive(Debug, Clone, PartialEq)]
pub enum CypherTemporal {
    /// `xsd:date` — days since 1970-01-01.
    Date { days: i64, iso: String },
    /// `xsd:dateTime` — UTC epoch seconds + subsecond nanos; the original
    /// offset in seconds when the lexical form carried one.
    DateTime {
        epoch_seconds: i64,
        nanos: u32,
        tz_offset_seconds: Option<i32>,
        iso: String,
    },
    /// `xsd:time` — nanoseconds since midnight; offset as for DateTime.
    Time {
        nanos_since_midnight: i64,
        tz_offset_seconds: Option<i32>,
        iso: String,
    },
}

/// The typed counterpart of [`super::cypher::table`]: same column
/// selection, same rows, typed cells. Async because node/relationship
/// cells fetch their properties from the view.
pub(crate) async fn typed_table(
    result: &QueryResult,
    compactor: &IriCompactor,
    view: &GraphDb,
) -> Result<(Vec<String>, Vec<Vec<CypherCell>>)> {
    if view.has_policy() {
        return Err(FormatError::InvalidBinding(
            "typed Cypher results are not supported under a view policy: format-time \
             node hydration would bypass per-flake policy filtering"
                .to_string(),
        ));
    }
    let col_vars = super::cypher::column_vars(result);
    let columns: Vec<String> = col_vars
        .iter()
        .map(|&v| result.vars.name(v).to_string())
        .collect();

    let mut hydrator = NodeHydrator::new(view, compactor);
    let mut rows = Vec::new();
    for batch in &result.batches {
        for row_idx in 0..batch.len() {
            let mut row = Vec::with_capacity(col_vars.len());
            for &var_id in &col_vars {
                let cell = match batch.get(row_idx, var_id) {
                    Some(b) => binding_cell(result, b, &mut hydrator).await?,
                    None => CypherCell::Value(JsonValue::Null),
                };
                row.push(cell);
            }
            rows.push(row);
        }
    }
    Ok((columns, rows))
}

fn binding_cell<'a>(
    result: &'a QueryResult,
    binding: &'a Binding,
    hydrator: &'a mut NodeHydrator<'_>,
) -> BoxFuture<'a, Result<CypherCell>> {
    async move {
        if binding.is_encoded() {
            let materialized = super::materialize::materialize_binding(result, binding)?;
            return binding_cell_owned(result, materialized, hydrator).await;
        }
        match binding {
            Binding::Unbound | Binding::Poisoned => Ok(CypherCell::Value(JsonValue::Null)),
            Binding::Sid { sid, .. } => Ok(CypherCell::Node(Box::new(hydrator.node(sid).await?))),
            Binding::IriMatch { primary_sid, .. } => Ok(CypherCell::Node(Box::new(
                hydrator.node(primary_sid).await?,
            ))),
            Binding::Rel(rel) => {
                let start_iri = hydrator.compactor.decode_sid(&rel.start)?;
                let end_iri = hydrator.compactor.decode_sid(&rel.end)?;
                let type_iri = hydrator.compactor.decode_sid(&rel.predicate)?;
                let (reifier_iri, properties) = match &rel.reifier {
                    Some(reifier) => (
                        Some(hydrator.compactor.decode_sid(reifier)?),
                        hydrator.annotation_properties(reifier).await?,
                    ),
                    None => (None, Vec::new()),
                };
                Ok(CypherCell::Relationship(Box::new(CypherRelationship {
                    start_iri,
                    end_iri,
                    type_name: cypher_name_from_iri(&type_iri),
                    reifier_iri,
                    properties,
                })))
            }
            Binding::Path { nodes, edges } => Ok(CypherCell::Path(Box::new(
                hydrator.path(nodes, edges).await?,
            ))),
            Binding::List(values) | Binding::Grouped(values) => {
                let mut cells = Vec::with_capacity(values.len());
                for v in values {
                    cells.push(binding_cell(result, v, hydrator).await?);
                }
                Ok(CypherCell::List(cells))
            }
            Binding::Map(entries) => {
                let mut cells = Vec::with_capacity(entries.len());
                for (k, v) in entries {
                    cells.push((k.to_string(), binding_cell(result, v, hydrator).await?));
                }
                Ok(CypherCell::Map(cells))
            }
            Binding::Lit { val, .. } => match val {
                FlakeValue::Decimal(d) => Ok(CypherCell::Decimal(d.to_plain_string())),
                FlakeValue::BigInt(n) => Ok(CypherCell::BigInt(n.to_string())),
                FlakeValue::Date(d) => Ok(CypherCell::Temporal(date_cell(d))),
                FlakeValue::DateTime(dt) => Ok(CypherCell::Temporal(datetime_cell(dt))),
                FlakeValue::Time(t) => Ok(CypherCell::Temporal(time_cell(t))),
                _ => Ok(CypherCell::Value(
                    super::jsonld::format_binding_with_result(result, binding, hydrator.compactor)?,
                )),
            },
            _ => Ok(CypherCell::Value(
                super::jsonld::format_binding_with_result(result, binding, hydrator.compactor)?,
            )),
        }
    }
    .boxed()
}

/// Owned-binding variant for post-materialization recursion.
async fn binding_cell_owned(
    result: &QueryResult,
    binding: Binding,
    hydrator: &mut NodeHydrator<'_>,
) -> Result<CypherCell> {
    binding_cell(result, &binding, hydrator).await
}

fn date_cell(d: &fluree_db_core::temporal::Date) -> CypherTemporal {
    CypherTemporal::Date {
        days: d.days_since_epoch() as i64,
        iso: d.original().to_string(),
    }
}

fn datetime_cell(dt: &fluree_db_core::temporal::DateTime) -> CypherTemporal {
    let micros = dt.epoch_micros();
    CypherTemporal::DateTime {
        epoch_seconds: micros.div_euclid(1_000_000),
        nanos: (micros.rem_euclid(1_000_000) * 1_000) as u32,
        tz_offset_seconds: dt.tz_offset().map(|o| o.local_minus_utc()),
        iso: dt.original().to_string(),
    }
}

fn time_cell(t: &fluree_db_core::temporal::Time) -> CypherTemporal {
    let whole_minutes_secs = (t.hours() as f64) * 3600.0 + (t.minutes() as f64) * 60.0;
    let nanos = (whole_minutes_secs + t.seconds()) * 1_000_000_000.0;
    CypherTemporal::Time {
        nanos_since_midnight: nanos.round() as i64,
        tz_offset_seconds: t.tz_offset().map(|o| o.local_minus_utc()),
        iso: t.original().to_string(),
    }
}

/// Fetches and caches node hydrations for one table walk.
struct NodeHydrator<'a> {
    view: &'a GraphDb,
    compactor: &'a IriCompactor,
    rdf_type: Option<Sid>,
    node_marker: Option<Sid>,
    cache: HashMap<Sid, CypherNode>,
}

impl<'a> NodeHydrator<'a> {
    fn new(view: &'a GraphDb, compactor: &'a IriCompactor) -> Self {
        Self {
            view,
            compactor,
            rdf_type: view.snapshot.encode_iri(fluree_vocab::rdf::TYPE),
            node_marker: view.snapshot.encode_iri(fluree_vocab::fluree::NODE),
            cache: HashMap::new(),
        }
    }

    async fn subject_flakes(&self, sid: &Sid) -> Result<Vec<fluree_db_core::Flake>> {
        self.view
            .as_graph_db_ref()
            .range_with_opts(
                IndexType::Spot,
                RangeTest::Eq,
                RangeMatch::subject(sid.clone()),
                RangeOptions::default(),
            )
            .await
            .map_err(|e| FormatError::InvalidBinding(format!("node property fetch failed: {e}")))
    }

    async fn node(&mut self, sid: &Sid) -> Result<CypherNode> {
        if let Some(hit) = self.cache.get(sid) {
            return Ok(hit.clone());
        }
        let iri = self.compactor.decode_sid(sid)?;
        let mut labels = Vec::new();
        let mut props: Vec<(String, Vec<CypherCell>)> = Vec::new();
        for flake in self.subject_flakes(sid).await? {
            if !flake.op {
                continue;
            }
            if Some(&flake.p) == self.rdf_type.as_ref() {
                if let FlakeValue::Ref(class_sid) = &flake.o {
                    if Some(class_sid) == self.node_marker.as_ref() {
                        continue;
                    }
                    let class_iri = self.compactor.decode_sid(class_sid)?;
                    labels.push(cypher_name_from_iri(&class_iri));
                }
                continue;
            }
            let p_iri = self.compactor.decode_sid(&flake.p)?;
            if p_iri.starts_with(FLUREE_SYSTEM_NS) {
                continue;
            }
            let key = cypher_name_from_iri(&p_iri);
            let cell = self.flake_value_cell(&flake.o)?;
            match props.iter_mut().find(|(k, _)| *k == key) {
                Some((_, cells)) => cells.push(cell),
                None => props.push((key, vec![cell])),
            }
        }
        let node = CypherNode {
            iri,
            labels,
            properties: props
                .into_iter()
                .map(|(k, mut cells)| {
                    let cell = if cells.len() == 1 {
                        cells.pop().expect("one cell")
                    } else {
                        CypherCell::List(cells)
                    };
                    (k, cell)
                })
                .collect(),
        };
        self.cache.insert(sid.clone(), node.clone());
        Ok(node)
    }

    /// Properties of a reifier (annotation) subject: user annotation keys
    /// only — the `db:reifies*` bookkeeping and `rdf:type` are skipped.
    async fn annotation_properties(&mut self, sid: &Sid) -> Result<Vec<(String, CypherCell)>> {
        let mut props: Vec<(String, CypherCell)> = Vec::new();
        for flake in self.subject_flakes(sid).await? {
            if !flake.op || Some(&flake.p) == self.rdf_type.as_ref() {
                continue;
            }
            let p_iri = self.compactor.decode_sid(&flake.p)?;
            if p_iri.starts_with(FLUREE_SYSTEM_NS) {
                continue;
            }
            props.push((
                cypher_name_from_iri(&p_iri),
                self.flake_value_cell(&flake.o)?,
            ));
        }
        Ok(props)
    }

    async fn path(&mut self, nodes: &[Sid], edges: &[(Sid, Sid, Sid)]) -> Result<CypherPath> {
        let mut path_nodes: Vec<CypherNode> = Vec::new();
        let mut node_index: HashMap<String, usize> = HashMap::new();
        let mut index_of_node = |n: CypherNode, path_nodes: &mut Vec<CypherNode>| -> usize {
            if let Some(&i) = node_index.get(&n.iri) {
                return i;
            }
            let i = path_nodes.len();
            node_index.insert(n.iri.clone(), i);
            path_nodes.push(n);
            i
        };

        let mut rels: Vec<CypherRelationship> = Vec::new();
        let mut indices = Vec::new();
        if nodes.is_empty() {
            return Ok(CypherPath {
                nodes: path_nodes,
                rels,
                indices,
            });
        }
        let first = self.node(&nodes[0]).await?;
        index_of_node(first, &mut path_nodes);

        for (hop, (s, p, o)) in edges.iter().enumerate() {
            let Some(walk_from) = nodes.get(hop) else {
                break;
            };
            let forward = s == walk_from;
            let start_iri = self.compactor.decode_sid(s)?;
            let end_iri = self.compactor.decode_sid(o)?;
            let type_iri = self.compactor.decode_sid(p)?;
            let rel = CypherRelationship {
                start_iri,
                end_iri,
                type_name: cypher_name_from_iri(&type_iri),
                reifier_iri: None,
                properties: Vec::new(),
            };
            let rel_pos = match rels.iter().position(|r| r == &rel) {
                Some(i) => i,
                None => {
                    rels.push(rel);
                    rels.len() - 1
                }
            };
            let rel_index = (rel_pos + 1) as i64;
            indices.push(if forward { rel_index } else { -rel_index });

            if let Some(next_sid) = nodes.get(hop + 1) {
                let next = self.node(next_sid).await?;
                let node_pos = index_of_node(next, &mut path_nodes);
                indices.push(node_pos as i64);
            }
        }
        Ok(CypherPath {
            nodes: path_nodes,
            rels,
            indices,
        })
    }

    fn flake_value_cell(&self, value: &FlakeValue) -> Result<CypherCell> {
        Ok(match value {
            FlakeValue::Ref(sid) => {
                CypherCell::Value(JsonValue::String(self.compactor.decode_sid(sid)?))
            }
            FlakeValue::String(s) => CypherCell::Value(JsonValue::String(s.to_string())),
            FlakeValue::Long(n) => CypherCell::Value(serde_json::json!(n)),
            FlakeValue::Double(d) => CypherCell::Value(if d.is_finite() {
                serde_json::json!(d)
            } else {
                JsonValue::String(d.to_string())
            }),
            FlakeValue::Boolean(b) => CypherCell::Value(serde_json::json!(b)),
            FlakeValue::Decimal(d) => CypherCell::Decimal(d.to_plain_string()),
            FlakeValue::BigInt(n) => CypherCell::BigInt(n.to_string()),
            FlakeValue::Date(d) => CypherCell::Temporal(date_cell(d)),
            FlakeValue::DateTime(dt) => CypherCell::Temporal(datetime_cell(dt)),
            FlakeValue::Time(t) => CypherCell::Temporal(time_cell(t)),
            FlakeValue::Json(s) => CypherCell::Value(
                serde_json::from_str(s).unwrap_or_else(|_| JsonValue::String(s.to_string())),
            ),
            FlakeValue::Vector(v) => CypherCell::Value(JsonValue::Array(
                v.iter().map(|f| serde_json::json!(f)).collect(),
            )),
            FlakeValue::Null => CypherCell::Value(JsonValue::Null),
            other => CypherCell::Value(JsonValue::String(other.to_string())),
        })
    }
}
