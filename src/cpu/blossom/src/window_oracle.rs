//! Re-export of the nostd window oracle with std support.
//!
//! The oracle logic lives in `micro_blossom_nostd::window_oracle` so that the
//! embedded (no_std) primal module can share the same artificial-syndrome and
//! observable-parity definitions.  This crate adds the toolchain's graph type
//! conversion helpers.

pub use micro_blossom_nostd::window_oracle::{
    commit_tentative, observable_parity, residual_syndrome, validate_window_layers, CommitResult,
    EdgeId, ObservableId, OracleEdge, OracleGraph,
};

use std::collections::BTreeMap;

use crate::resources::MicroBlossomSingle;
use micro_blossom_nostd::window::WindowSpec;

/// Convert a `MicroBlossomSingle` + window specs into the oracle's graph view.
///
/// The oracle needs for each global edge: its two endpoint detectors (or
/// `None` for a boundary), the list of observables it flips, and its `t_mid`.
/// For the prototype we store a single observable per edge via
/// `edge_observables: Vec<Option<usize>>` (aligned with
/// `MicroBlossomSingle::weighted_edges`).  Multi-observable edges are
/// represented by multiple entries in `OracleEdge::observables`.
pub fn oracle_graph_from_micro_blossom(
    global: &MicroBlossomSingle,
    edge_observables: &[Option<usize>],
    num_observables: usize,
) -> OracleGraph {
    assert_eq!(global.weighted_edges.len(), edge_observables.len());
    let is_virtual = |v: usize| global.virtual_vertices.contains(&v);
    let mut edges = Vec::new();
    for (idx, e) in global.weighted_edges.iter().enumerate() {
        let a_virt = is_virtual(e.l);
        let b_virt = is_virtual(e.r);
        let endpoints = match (a_virt, b_virt) {
            (false, false) => (Some(e.l), Some(e.r)),
            (false, true) => (Some(e.l), None),
            (true, false) => (Some(e.r), None),
            (true, true) => (None, None),
        };
        let t_a = global.positions.get(e.l).map(|p| p.t).unwrap_or(0.0);
        let t_b = global.positions.get(e.r).map(|p| p.t).unwrap_or(0.0);
        let t_mid = (t_a + t_b) / 2.0;
        let observables = if let Some(obs) = edge_observables[idx] {
            vec![obs]
        } else {
            vec![]
        };
        edges.push(OracleEdge {
            endpoints,
            observables,
            t_mid,
        });
    }
    // t per detector (for seam detection)
    let detector_t = global.positions.iter().map(|p| p.t).collect();
    OracleGraph {
        num_detectors: global.vertex_num,
        num_observables,
        detector_t,
        edges,
    }
}

/// Run the full two-layer commit and return the oracle's reference result.
///
/// This is the *software reference* for testing: it takes the tentative
/// corrections that would have been produced by decoding each window in
/// isolation (as a map `window_id -> Vec<global_edge_id>`) and applies the
/// exact same commit rule as the hardware-aware decoder in
/// `parallel_window::ParallelWindowDecoder`.  The result's
/// `residual_detectors` should be empty after both layers for a correct
/// decode.
pub fn oracle_two_layer_commit(
    tentative_by_window: &BTreeMap<String, Vec<EdgeId>>,
    windows: &[WindowSpec],
    graph: &OracleGraph,
    input_syndrome: &[usize],
) -> CommitResult {
    commit_tentative(tentative_by_window, windows, graph, input_syndrome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use micro_blossom_nostd::window::{WindowBoundary, WindowLayer, WindowSpec};

    #[test]
    fn oracle_from_graph_roundtrip() {
        // Minimal graph: 2 detectors, 1 edge, 1 observable
        let init = fusion_blossom::util::SolverInitializer::new(3, vec![(0, 1, 10), (1, 2, 10)], vec![2]);
        let pos = vec![
            fusion_blossom::visualize::VisualizePosition::new(0.0, 0.0, 0.0),
            fusion_blossom::visualize::VisualizePosition::new(0.0, 0.0, 1.0),
            fusion_blossom::visualize::VisualizePosition::new(0.0, 0.0, 0.0),
        ];
        let graph = MicroBlossomSingle::new(&init, &pos);
        let edge_obs = vec![Some(0), None];
        let og = oracle_graph_from_micro_blossom(&graph, &edge_obs, 1);
        assert_eq!(og.edges.len(), 2);
        assert_eq!(og.edges[0].observables, vec![0]);
    }
}
