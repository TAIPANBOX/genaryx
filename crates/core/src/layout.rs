//! Deterministic, seeded force-directed layout for the delegation graph.
//!
//! Architecture position (PHASE3 §5.3, decided): the graph LAYOUT is computed
//! in genaryx-core; both shells are dumb Canvas2D renderers of the result. This
//! avoids the WebGL parity trap (WebGL exists in the Tauri webview but not
//! natively in SwiftUI) and keeps the pilot-scale graph (tens-to-hundreds of
//! agents) trivial to draw. WebGL is only revisited if a bench proves Canvas2D
//! insufficient.
//!
//! Determinism is a hard requirement, exactly like the site sims' rule ("pure
//! function of the seed, never `Math.random`/wall-clock"): given the same
//! [`GraphView`] and [`LayoutConfig`] this produces byte-identical positions, so
//! a shell can recompute a layout and get the same picture, and a test can
//! assert equality. The only nondeterminism is cross-platform floating point;
//! on one machine it is exact. No wall-clock, no thread-order dependence (all
//! forces are summed single-threaded into a displacement buffer, then applied).

use crate::graph::{GraphEdge, GraphView, NodeKind};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Knobs for [`layout`]. Defaults suit the pilot scale (a few hundred nodes).
#[derive(Debug, Clone, Copy)]
pub struct LayoutConfig {
    pub iterations: u32,
    pub seed: u64,
    pub width: f64,
    pub height: f64,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            iterations: 300,
            seed: 0x5EED_C0DE,
            width: 1000.0,
            height: 1000.0,
        }
    }
}

/// A graph node with a computed position, ready for a Canvas2D renderer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PositionedNode {
    pub id: String,
    pub kind: NodeKind,
    pub event_count: u64,
    pub x: f64,
    pub y: f64,
}

/// The full laid-out graph the shells render: positioned nodes, the edges, and
/// the canvas bounds the positions live in.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LayoutView {
    pub nodes: Vec<PositionedNode>,
    pub edges: Vec<GraphEdge>,
    pub width: f64,
    pub height: f64,
}

/// A tiny deterministic PRNG (splitmix64) - used only to place the initial ring
/// jitter, never in the force loop, so the whole layout is a pure function of
/// `(view, cfg)`.
struct SplitMix64(u64);

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self(seed ^ 0x9E37_79B9_7F4A_7C15)
    }
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// A float in `[0, 1)`.
    fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / ((1u64 << 53) as f64)
    }
}

/// Compute a position for every node, keyed by node id. A Fruchterman-Reingold
/// force layout: nodes repel, edges attract, temperature cools each iteration.
/// Deterministic for a given `(view, cfg)`.
pub fn layout(view: &GraphView, cfg: &LayoutConfig) -> BTreeMap<String, (f64, f64)> {
    let n = view.nodes.len();
    let mut out = BTreeMap::new();
    if n == 0 {
        return out;
    }

    let (cx, cy) = (cfg.width / 2.0, cfg.height / 2.0);
    if n == 1 {
        out.insert(view.nodes[0].id.clone(), (cx, cy));
        return out;
    }

    // A stable node order (by id) so the layout does not depend on the incoming
    // Vec order; `GraphView::nodes` is already id-sorted, but do not rely on it.
    let mut ids: Vec<&str> = view.nodes.iter().map(|nd| nd.id.as_str()).collect();
    ids.sort_unstable();
    let index: BTreeMap<&str, usize> = ids.iter().enumerate().map(|(i, id)| (*id, i)).collect();

    // Initial positions: a ring plus a small seeded radial jitter, so two nodes
    // never start exactly coincident (which would make repulsion undefined).
    let mut rng = SplitMix64::new(cfg.seed);
    let base_r = cfg.width.min(cfg.height) * 0.4;
    let mut pos: Vec<(f64, f64)> = Vec::with_capacity(n);
    for i in 0..n {
        let angle = std::f64::consts::TAU * (i as f64) / (n as f64);
        let r = base_r * (0.6 + 0.5 * rng.unit());
        pos.push((cx + r * angle.cos(), cy + r * angle.sin()));
    }

    // Edges as index pairs (skip any endpoint not in the node set, defensively).
    let edges: Vec<(usize, usize)> = view
        .edges
        .iter()
        .filter_map(|e| Some((*index.get(e.from.as_str())?, *index.get(e.to.as_str())?)))
        .collect();

    let area = cfg.width * cfg.height;
    let k = (area / n as f64).sqrt(); // ideal edge length
    let mut temp = cfg.width * 0.1;
    let cooling = 0.95;
    let eps = 1e-4;

    for _ in 0..cfg.iterations {
        let mut disp = vec![(0.0_f64, 0.0_f64); n];

        // Repulsion between every pair.
        for i in 0..n {
            for j in (i + 1)..n {
                let mut dx = pos[i].0 - pos[j].0;
                let mut dy = pos[i].1 - pos[j].1;
                let mut d = (dx * dx + dy * dy).sqrt();
                if d < eps {
                    // Deterministic separation (never a random or a panic on d=0).
                    dx = 0.01 * (i as f64 + 1.0);
                    dy = 0.01 * (j as f64 + 1.0);
                    d = (dx * dx + dy * dy).sqrt();
                }
                let f = k * k / d;
                let (ux, uy) = (dx / d, dy / d);
                disp[i].0 += ux * f;
                disp[i].1 += uy * f;
                disp[j].0 -= ux * f;
                disp[j].1 -= uy * f;
            }
        }

        // Attraction along edges.
        for &(a, b) in &edges {
            let dx = pos[a].0 - pos[b].0;
            let dy = pos[a].1 - pos[b].1;
            let d = (dx * dx + dy * dy).sqrt().max(eps);
            let f = d * d / k;
            let (ux, uy) = (dx / d, dy / d);
            disp[a].0 -= ux * f;
            disp[a].1 -= uy * f;
            disp[b].0 += ux * f;
            disp[b].1 += uy * f;
        }

        // Apply, capped by the current temperature, clamped to the canvas.
        for i in 0..n {
            let dl = (disp[i].0 * disp[i].0 + disp[i].1 * disp[i].1).sqrt();
            if dl > eps {
                let cap = dl.min(temp);
                pos[i].0 += disp[i].0 / dl * cap;
                pos[i].1 += disp[i].1 / dl * cap;
            }
            pos[i].0 = pos[i].0.clamp(0.0, cfg.width);
            pos[i].1 = pos[i].1.clamp(0.0, cfg.height);
        }

        temp *= cooling;
    }

    for (id, &(x, y)) in ids.iter().zip(pos.iter()) {
        out.insert((*id).to_string(), (x, y));
    }
    out
}

/// Convenience: run [`layout`] and fold the positions back into the graph view,
/// producing the single [`LayoutView`] a shell's Canvas2D renderer consumes.
pub fn layout_view(view: &GraphView, cfg: &LayoutConfig) -> LayoutView {
    let pos = layout(view, cfg);
    let nodes = view
        .nodes
        .iter()
        .map(|nd| {
            let (x, y) = pos.get(&nd.id).copied().unwrap_or((0.0, 0.0));
            PositionedNode {
                id: nd.id.clone(),
                kind: nd.kind,
                event_count: nd.event_count,
                x,
                y,
            }
        })
        .collect();
    LayoutView {
        nodes,
        edges: view.edges.clone(),
        width: cfg.width,
        height: cfg.height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{AgentEvent, SchemaVersion};
    use crate::graph::DelegationGraph;
    use serde_json::Map;

    fn ev(agent_id: &str, obo: &[&str]) -> AgentEvent {
        AgentEvent {
            schema: SchemaVersion::SCHEMA_V0_2.to_string(),
            ts: "2026-01-02T00:00:01Z".to_string(),
            source: "wardryx".to_string(),
            event_type: "policy_allow".to_string(),
            agent_id: agent_id.to_string(),
            severity: None,
            run_id: None,
            on_behalf_of: obo.iter().map(|s| s.to_string()).collect(),
            data: None,
            prev_hash: None,
            extra: Map::new(),
        }
    }

    fn sample_view() -> GraphView {
        let mut g = DelegationGraph::new();
        g.add_event(&ev(
            "agent://acme/w1",
            &["user://acme/a", "agent://acme/orch"],
        ));
        g.add_event(&ev(
            "agent://acme/w2",
            &["user://acme/a", "agent://acme/orch"],
        ));
        g.add_event(&ev("agent://acme/w3", &["user://acme/b"]));
        g.view()
    }

    #[test]
    fn deterministic_for_same_seed() {
        let v = sample_view();
        let cfg = LayoutConfig::default();
        let a = layout(&v, &cfg);
        let b = layout(&v, &cfg);
        assert_eq!(a, b, "same (view,cfg) must yield identical positions");
    }

    #[test]
    fn every_node_positioned_in_bounds_and_finite() {
        let v = sample_view();
        let cfg = LayoutConfig::default();
        let pos = layout(&v, &cfg);
        assert_eq!(pos.len(), v.nodes.len());
        for (id, (x, y)) in &pos {
            assert!(
                x.is_finite() && y.is_finite(),
                "{id} has a non-finite position"
            );
            assert!(*x >= 0.0 && *x <= cfg.width, "{id} x out of bounds");
            assert!(*y >= 0.0 && *y <= cfg.height, "{id} y out of bounds");
        }
    }

    #[test]
    fn empty_and_single_node() {
        let empty = layout(&GraphView::default(), &LayoutConfig::default());
        assert!(empty.is_empty());

        let mut g = DelegationGraph::new();
        g.add_event(&ev("agent://acme/solo", &[]));
        let pos = layout(&g.view(), &LayoutConfig::default());
        assert_eq!(pos.len(), 1);
        // a lone node sits at the canvas center
        let (x, y) = pos["agent://acme/solo"];
        assert_eq!((x, y), (500.0, 500.0));
    }

    #[test]
    fn layout_view_carries_kind_and_positions() {
        let v = sample_view();
        let lv = layout_view(&v, &LayoutConfig::default());
        assert_eq!(lv.nodes.len(), v.nodes.len());
        assert_eq!(lv.edges.len(), v.edges.len());
        assert_eq!((lv.width, lv.height), (1000.0, 1000.0));
        // round-trips as JSON (the shape that crosses to the shells)
        let json = serde_json::to_string(&lv).unwrap();
        let back: LayoutView = serde_json::from_str(&json).unwrap();
        assert_eq!(lv, back);
    }
}
