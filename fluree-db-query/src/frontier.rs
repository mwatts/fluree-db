//! Level-batched frontier expansion for path traversals.
//!
//! Per-node `range_with_overlay` probes make BFS cost `visited × per-call
//! overhead` (cursor setup, overlay merge, flake materialization — tens of
//! µs each). This module expands a whole frontier level at once:
//!
//! - **Base edges** come from the gap-aware batched index sweeps
//!   ([`batched_lookup_predicate_refs`] /
//!   [`batched_lookup_subject_properties`] /
//!   [`batched_lookup_inbound_refs`]), which decode each touched leaflet
//!   once for the level instead of once per node.
//! - **Novelty** is applied as an *edge delta* built from one overlay walk
//!   (cached process-wide on `content_version`): asserted ref-edges extend
//!   the expansion, retractions mask base rows. There is no "overlay-free"
//!   gate to silently close — the delta path is merge-correct.
//!
//! Nodes are [`PathNode`]s: persisted subjects travel as raw `u64` ids
//! (cheap hash, no string work); novelty-only subjects (no persisted id)
//! travel as `Sid`s. Callers resolve `Base` ids back to `Sid`s only for
//! emitted results.

use std::sync::Arc;

use fluree_db_binary_index::read::batched_lookup::{
    batched_lookup_inbound_refs, batched_lookup_predicate_refs, batched_lookup_subject_properties,
};
use fluree_db_binary_index::BinaryIndexStore;
use fluree_db_core::o_type::OType;
use fluree_db_core::{IndexType, OverlayProvider, Sid};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::error::{QueryError, Result};
use crate::property_path::is_reserved_edge_predicate;
use crate::ExecutionContext;

/// A traversal node: a persisted subject by raw id, or a novelty-only
/// subject by Sid.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum PathNode {
    Base(u64),
    Novel(Sid),
}

/// Ref-edge changes the overlay contributes, netted per `(s, p, o)` at the
/// query's `to_t`. Base-resolvable retractions mask batched base rows;
/// asserts extend the expansion (and are the only way novelty-only nodes
/// enter a traversal).
#[derive(Debug, Default)]
pub(crate) struct OverlayEdgeDelta {
    fwd_add: FxHashMap<PathNode, Vec<PathNode>>,
    bwd_add: FxHashMap<PathNode, Vec<PathNode>>,
    /// Retracted base edges as `(s_id, p_id, o_id)`.
    del: FxHashSet<(u64, u32, u64)>,
}

/// Expands frontier levels for one traversal: fixed graph, time bound, and
/// predicate filter (typed) or reserved-predicate exclusion (wildcard).
pub(crate) struct FrontierExpander {
    store: Option<Arc<BinaryIndexStore>>,
    g_id: u16,
    to_t: i64,
    /// Typed traversal: the predicate's index id (`None` when the
    /// predicate has no indexed facts — base lane contributes nothing).
    p_id: Option<u32>,
    /// Wildcard traversal (no predicate constraint).
    wildcard: bool,
    /// Wildcard: p_ids never traversed (`rdf:type`, `f:reifies*`).
    reserved_pids: FxHashSet<u32>,
    delta: Arc<OverlayEdgeDelta>,
}

impl FrontierExpander {
    /// Build an expander for the context's single active graph.
    /// `predicate` fixes a typed traversal; `None` is the wildcard form.
    pub(crate) fn new(ctx: &ExecutionContext<'_>, predicate: Option<&Sid>) -> Result<Self> {
        let (_db, overlay, to_t) = ctx.require_single_graph()?;
        let store = ctx.binary_store.as_ref().map(Arc::clone);
        let g_id = ctx.binary_g_id;

        let (p_id, wildcard, reserved_pids) = match predicate {
            Some(pred) => (
                store.as_ref().and_then(|s| s.sid_to_p_id(pred)),
                false,
                FxHashSet::default(),
            ),
            None => {
                let mut reserved = FxHashSet::default();
                if let Some(store) = &store {
                    for iri in [
                        fluree_vocab::rdf::TYPE,
                        fluree_vocab::reifies_iris::GRAPH,
                        fluree_vocab::reifies_iris::SUBJECT,
                        fluree_vocab::reifies_iris::PREDICATE,
                        fluree_vocab::reifies_iris::OBJECT,
                        fluree_vocab::reifies_iris::DATATYPE,
                        fluree_vocab::reifies_iris::LANG,
                        fluree_vocab::reifies_iris::LIST_INDEX,
                    ] {
                        if let Some(sid) = ctx.active_snapshot.encode_iri(iri) {
                            if let Some(p_id) = store.sid_to_p_id(&sid) {
                                reserved.insert(p_id);
                            }
                        }
                    }
                }
                (None, true, reserved)
            }
        };

        let delta = overlay_edge_delta(overlay, store.as_deref(), g_id, to_t, predicate);
        Ok(Self {
            store,
            g_id,
            to_t,
            p_id,
            wildcard,
            reserved_pids,
            delta,
        })
    }

    /// Convert a Sid endpoint into a [`PathNode`] (persisted id when the
    /// dictionary knows it).
    pub(crate) fn path_node(&self, sid: &Sid) -> PathNode {
        if let Some(store) = &self.store {
            if let Ok(Some(s_id)) = store.find_subject_id_by_parts(sid.namespace_code, &sid.name) {
                return PathNode::Base(s_id);
            }
        }
        PathNode::Novel(sid.clone())
    }

    /// Resolve a [`PathNode`] back to a Sid for emission.
    pub(crate) fn sid_of(&self, ctx: &ExecutionContext<'_>, node: &PathNode) -> Result<Sid> {
        match node {
            PathNode::Novel(sid) => Ok(sid.clone()),
            PathNode::Base(s_id) => {
                let resolved = ctx.resolve_subject_iri(*s_id).ok_or_else(|| {
                    QueryError::Internal(format!("unknown subject id {s_id} in path result"))
                })?;
                let iri = resolved
                    .map_err(|e| QueryError::Internal(format!("resolve subject id {s_id}: {e}")))?;
                ctx.active_snapshot.encode_iri(&iri).ok_or_else(|| {
                    QueryError::Internal(format!("re-encode subject iri {iri} failed"))
                })
            }
        }
    }

    /// Expand one frontier level. Returns `(parent, neighbor)` pairs;
    /// `forward` follows edges source→target, else target→source.
    pub(crate) fn expand(
        &self,
        ctx: &ExecutionContext<'_>,
        frontier: &[PathNode],
        forward: bool,
    ) -> Result<Vec<(PathNode, PathNode)>> {
        ctx.tracker.consume_fuel(frontier.len() as u64)?;
        let mut pairs: Vec<(PathNode, PathNode)> = Vec::new();

        if let Some(store) = &self.store {
            let base_ids: Vec<u64> = frontier
                .iter()
                .filter_map(|n| match n {
                    PathNode::Base(id) => Some(*id),
                    PathNode::Novel(_) => None,
                })
                .collect();
            if !base_ids.is_empty() {
                if forward {
                    self.expand_base_forward(store, &base_ids, &mut pairs)?;
                } else {
                    self.expand_base_backward(store, &base_ids, &mut pairs)?;
                }
            }
        }

        // Overlay-asserted edges, for every frontier node (persisted or
        // novelty-only).
        let adds = if forward {
            &self.delta.fwd_add
        } else {
            &self.delta.bwd_add
        };
        if !adds.is_empty() {
            for node in frontier {
                if let Some(neighbors) = adds.get(node) {
                    for neighbor in neighbors {
                        pairs.push((node.clone(), neighbor.clone()));
                    }
                }
            }
        }
        Ok(pairs)
    }

    fn expand_base_forward(
        &self,
        store: &Arc<BinaryIndexStore>,
        base_ids: &[u64],
        pairs: &mut Vec<(PathNode, PathNode)>,
    ) -> Result<()> {
        if self.wildcard {
            let rows = batched_lookup_subject_properties(store, self.g_id, base_ids, self.to_t)
                .map_err(|e| QueryError::Internal(format!("batched frontier spot: {e}")))?;
            for (s_id, props) in rows {
                for (p_id, o_type, o_key) in props {
                    if !OType::from_u16(o_type).is_node_ref()
                        || self.reserved_pids.contains(&p_id)
                        || self.delta.del.contains(&(s_id, p_id, o_key))
                    {
                        continue;
                    }
                    pairs.push((PathNode::Base(s_id), PathNode::Base(o_key)));
                }
            }
        } else if let Some(p_id) = self.p_id {
            let rows = batched_lookup_predicate_refs(store, self.g_id, p_id, base_ids, self.to_t)
                .map_err(|e| QueryError::Internal(format!("batched frontier psot: {e}")))?;
            for (s_id, targets) in rows {
                for o_id in targets {
                    if self.delta.del.contains(&(s_id, p_id, o_id)) {
                        continue;
                    }
                    pairs.push((PathNode::Base(s_id), PathNode::Base(o_id)));
                }
            }
        }
        Ok(())
    }

    fn expand_base_backward(
        &self,
        store: &Arc<BinaryIndexStore>,
        base_ids: &[u64],
        pairs: &mut Vec<(PathNode, PathNode)>,
    ) -> Result<()> {
        let rows = batched_lookup_inbound_refs(store, self.g_id, base_ids, self.to_t)
            .map_err(|e| QueryError::Internal(format!("batched frontier opst: {e}")))?;
        for (o_id, inbound) in rows {
            for (p_id, s_id) in inbound {
                let keep = if self.wildcard {
                    !self.reserved_pids.contains(&p_id)
                } else {
                    Some(p_id) == self.p_id
                };
                if !keep || self.delta.del.contains(&(s_id, p_id, o_id)) {
                    continue;
                }
                pairs.push((PathNode::Base(o_id), PathNode::Base(s_id)));
            }
        }
        Ok(())
    }
}

/// One netted overlay walk → edge delta, cached process-wide per
/// `(content_version, g_id, predicate, to_t)`. Overlays without a version
/// stamp build fresh (still one walk per query, never per hop).
fn overlay_edge_delta(
    overlay: &dyn OverlayProvider,
    store: Option<&BinaryIndexStore>,
    g_id: u16,
    to_t: i64,
    predicate: Option<&Sid>,
) -> Arc<OverlayEdgeDelta> {
    use std::sync::OnceLock;
    type Key = (u64, u16, Option<Sid>, i64);
    type DeltaCache = std::sync::Mutex<lru::LruCache<Key, Arc<OverlayEdgeDelta>>>;
    static CACHE: OnceLock<DeltaCache> = OnceLock::new();
    static EMPTY: OnceLock<Arc<OverlayEdgeDelta>> = OnceLock::new();

    if overlay.is_effectively_empty() {
        return Arc::clone(EMPTY.get_or_init(|| Arc::new(OverlayEdgeDelta::default())));
    }
    let version = overlay.content_version();
    if let Some(version) = version {
        let key = (version, g_id, predicate.cloned(), to_t);
        let cache = CACHE.get_or_init(|| {
            std::sync::Mutex::new(lru::LruCache::new(
                std::num::NonZeroUsize::new(16).expect("nonzero"),
            ))
        });
        if let Some(hit) = cache.lock().expect("delta cache poisoned").get(&key) {
            return Arc::clone(hit);
        }
        let delta = Arc::new(build_overlay_edge_delta(
            overlay, store, g_id, to_t, predicate,
        ));
        cache
            .lock()
            .expect("delta cache poisoned")
            .put(key, Arc::clone(&delta));
        delta
    } else {
        Arc::new(build_overlay_edge_delta(
            overlay, store, g_id, to_t, predicate,
        ))
    }
}

fn build_overlay_edge_delta(
    overlay: &dyn OverlayProvider,
    store: Option<&BinaryIndexStore>,
    g_id: u16,
    to_t: i64,
    predicate: Option<&Sid>,
) -> OverlayEdgeDelta {
    // Net per (s, p, o): the highest-t op within to_t wins.
    let mut netted: FxHashMap<(Sid, Sid, Sid), (i64, bool)> = FxHashMap::default();
    overlay.for_each_overlay_flake(
        g_id,
        IndexType::Spot,
        None,
        None,
        true,
        to_t,
        &mut |flake| {
            let fluree_db_core::FlakeValue::Ref(obj) = &flake.o else {
                return;
            };
            match predicate {
                Some(pred) => {
                    if flake.p != *pred {
                        return;
                    }
                }
                None => {
                    if is_reserved_edge_predicate(&flake.p) {
                        return;
                    }
                }
            }
            let key = (flake.s.clone(), flake.p.clone(), obj.clone());
            let entry = netted.entry(key).or_insert((flake.t, flake.op));
            if flake.t >= entry.0 {
                *entry = (flake.t, flake.op);
            }
        },
    );

    let resolve = |sid: &Sid| -> PathNode {
        if let Some(store) = store {
            if let Ok(Some(s_id)) = store.find_subject_id_by_parts(sid.namespace_code, &sid.name) {
                return PathNode::Base(s_id);
            }
        }
        PathNode::Novel(sid.clone())
    };

    let mut delta = OverlayEdgeDelta::default();
    for ((s, p, o), (_t, op)) in netted {
        let s_node = resolve(&s);
        let o_node = resolve(&o);
        if op {
            delta
                .fwd_add
                .entry(s_node.clone())
                .or_default()
                .push(o_node.clone());
            delta.bwd_add.entry(o_node).or_default().push(s_node);
        } else if let (PathNode::Base(s_id), PathNode::Base(o_id)) = (&s_node, &o_node) {
            // A retraction can only mask a persisted base edge.
            if let Some(p_id) = store.and_then(|st| st.sid_to_p_id(&p)) {
                delta.del.insert((*s_id, p_id, *o_id));
            }
        }
    }
    delta
}
