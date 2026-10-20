//! Residual-syndrome and global-prediction oracle
//!
//! This module implements the **software oracle** described in the plan's
//! Sprint 1 exit gate.  It does *not* run a decoder; it checks the algebra
//! that a correct parallel-window decoder must satisfy.  Given a global
//! decoding graph `G = (V,E,w,obs)` and a set of tentative corrections
//! (subgraphs) produced by window decoders, the oracle:
//!
//! 1. Determines which edges are *committed* (inside a window's commit region)
//!    and which are discarded.
//! 2. Computes the **residual syndrome** — the set of artificial detection
//!    events on the commit/buffer seam plus any uncorrected original defects
//!    inside the gap — that is passed to the next layer.
//! 3. Computes the **global observable parity** as the XOR of `obs(e)` over all
//!    committed edges.  Both layer-0 and layer-1 decoders commit to this
//!    global prediction; there is a single logical correction, not two.
//!
//! The oracle mirrors the arithmetic in `Graph::evaluate_correction` from the
//! toolchain crate (`src/lib.rs`) but operates on a windowed graph partition.
//! It is the reference for testing the Rust decoder and for validating that
//! seam logic, physical vs temporal boundaries, and observable accounting are
//! correct before any RTL changes.

use std::collections::{BTreeMap, BTreeSet};

use crate::window::{WindowBoundary, WindowLayer, WindowSpec};

/// Edge identifier in the *global* graph.  For the toolchain this is
/// `edge_metadata` index; for micro-blossom it is `weighted_edges` index.
pub type EdgeId = usize;
/// Detector identifier (global vertex index for non-virtual detectors).
pub type DetectorId = usize;
/// Observable identifier (logical qubit / frame).
pub type ObservableId = usize;

/// Minimal description of a global edge needed by the oracle.
#[derive(Debug, Clone)]
pub struct OracleEdge {
    /// Endpoints as detector IDs.  `None` denotes the boundary (virtual node)
    /// — an edge to `None` flips exactly one detector.
    pub endpoints: (Option<DetectorId>, Option<DetectorId>),
    /// Observable(s) flipped by this error mechanism.  Empty = no logical effect.
    pub observables: Vec<ObservableId>,
    /// Midpoint `t` coordinate of the edge (average of endpoint `t`).  Used to
    /// decide commit vs buffer membership.  For boundary edges the virtual
    /// endpoint is assigned the window's boundary `t`.
    pub t_mid: f64,
}

/// Global graph view required by the oracle.
#[derive(Debug, Clone)]
pub struct OracleGraph {
    pub num_detectors: usize,
    pub num_observables: usize,
    /// Detector positions: `[i,j,t]` per detector id (virtual detectors omitted).
    pub detector_t: Vec<f64>,
    /// Edges in global order.
    pub edges: Vec<OracleEdge>,
}

/// Result of committing a set of window corrections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitResult {
    /// Global edges that were committed (inside some window's commit region).
    pub committed_edges: Vec<EdgeId>,
    /// Global observable parity after all layers (XOR of `observables` along
    /// committed edges).
    pub observable_parity: Vec<ObservableId>,
    /// Residual detectors that must be decoded in the next layer: detectors
    /// whose parity is `1` after applying only the committed edges.  For
    /// layer-0 this is the *artificial* syndrome on the seam; for a complete
    /// two-layer decode it should be empty.
    pub residual_detectors: Vec<DetectorId>,
}

/// Compute `input_syndrome XOR boundary(committed_edges)`.
///
/// `input_syndrome` is the set of detectors with odd syndrome (defects).
/// `committed_edges` is the set of global edge ids whose correction has been
/// committed.  The boundary of an edge is the set of its endpoint detectors
/// (0, 1 or 2 of them).  Repeated edges cancel via XOR.
pub fn residual_syndrome(
    input_syndrome: &[DetectorId],
    committed_edges: &[EdgeId],
    graph: &OracleGraph,
) -> Vec<DetectorId> {
    let mut residual: BTreeSet<DetectorId> = input_syndrome.iter().copied().collect();
    // Validate uniqueness as the toolchain does.
    debug_assert!(
        {
            let mut sorted = input_syndrome.to_vec();
            sorted.sort_unstable();
            sorted.windows(2).all(|w| w[0] != w[1])
        },
        "duplicate input defects"
    );
    for &eid in committed_edges {
        let edge = &graph.edges[eid];
        for endpoint in [edge.endpoints.0, edge.endpoints.1] {
            if let Some(det) = endpoint {
                toggle(&mut residual, det);
            }
        }
    }
    residual.into_iter().collect()
}

/// XOR observable labels along committed edges.
pub fn observable_parity(committed_edges: &[EdgeId], graph: &OracleGraph) -> Vec<ObservableId> {
    let mut parity: BTreeSet<ObservableId> = BTreeSet::new();
    for &eid in committed_edges {
        for &obs in &graph.edges[eid].observables {
            toggle(&mut parity, obs);
        }
    }
    parity.into_iter().collect()
}

fn toggle(set: &mut BTreeSet<usize>, value: usize) {
    if !set.insert(value) {
        set.remove(&value);
    }
}

/// Partition tentative corrections into committed vs discarded and compute the
/// combined global parity.  `tentative_by_window` maps window id → list of
/// global edge ids that the inner decoder returned for that window (the full
/// subgraph, including buffer edges).  The oracle keeps an edge iff its
/// `t_mid` lies inside the window's commit interval.
///
/// This is exactly the rule from the project vision:
///
/// > "observable flips in the commit region are 'committed' to the global
/// > prediction, while observable flips in the buffer region are discarded.
/// > Errors crossing the commit and buffer seam are used to generate residual
/// > syndromes which contain artificial detection events, or detectors in
/// > those crossing errors that sit on the seam."
///
/// Crossing edges are handled naturally: their `t_mid` is on the seam, so they
/// are either just inside or just outside the commit interval and contribute
/// precisely one artificial detector at the seam endpoint outside the commit.
pub fn commit_tentative(
    tentative_by_window: &BTreeMap<String, Vec<EdgeId>>,
    windows: &[WindowSpec],
    graph: &OracleGraph,
    input_syndrome: &[DetectorId],
) -> CommitResult {
    let window_by_id: BTreeMap<&str, &WindowSpec> = windows.iter().map(|w| (w.id.as_str(), w)).collect();
    let mut committed: Vec<EdgeId> = Vec::new();
    // Track which detectors are flipped an odd number of times by committed edges
    // to compute residual at the seam.  We cannot just union tentative edges and
    // filter, because two windows may commit edges that cancel (same physical
    // edge committed twice would be a bug — overlapping commits are forbidden
    // by validation).  We therefore assert disjoint commits.
    let mut commit_t_set: BTreeSet<EdgeId> = BTreeSet::new();
    for (win_id, edge_ids) in tentative_by_window {
        let win = window_by_id
            .get(win_id.as_str())
            .unwrap_or_else(|| panic!("window id {} not in spec", win_id));
        for &eid in edge_ids {
            let edge = &graph.edges[eid];
            let inside_commit = edge.t_mid >= win.commit_start && edge.t_mid < win.commit_end;
            if inside_commit {
                if !commit_t_set.insert(eid) {
                    // Same global edge committed by two windows would imply overlapping
                    // commit regions, which validation forbids.
                    panic!("edge {} committed by multiple windows (overlapping commits)", eid);
                }
                committed.push(eid);
            }
            // Edges outside commit are discarded — they do not affect global
            // prediction and do not generate artificial events directly.  The
            // artificial event arises from the *committed* edge's endpoint that
            // lies on the seam (outside commit), which is captured by
            // residual_syndrome below.
        }
    }

    // Sort for deterministic output.
    committed.sort_unstable();
    committed.dedup();

    let residual = residual_syndrome(input_syndrome, &committed, graph);
    let parity = observable_parity(&committed, graph);

    CommitResult {
        committed_edges: committed,
        observable_parity: parity,
        residual_detectors: residual,
    }
}

/// Validate that window commits are disjoint and that layer-0 windows are
/// open / layer-1 closed as required by the vision.
pub fn validate_window_layers(windows: &[WindowSpec]) -> Result<(), String> {
    for w in windows {
        w.validate()?;
    }
    for w in windows {
        match w.layer {
            WindowLayer::Top => {
                if w.boundary != WindowBoundary::Open {
                    return Err(format!("top window {} must be temporally open", w.id));
                }
            }
            WindowLayer::Bottom => {
                if w.boundary != WindowBoundary::Closed {
                    return Err(format!("bottom window {} must be temporally closed", w.id));
                }
                if (w.commit_start - w.t_start).abs() > 1e-9
                    || (w.commit_end - w.t_end).abs() > 1e-9
                {
                    return Err(format!(
                        "bottom window {} must be fully committed (no buffer)",
                        w.id
                    ));
                }
            }
        }
    }
    // Same-layer commits must not overlap.
    for layer in [WindowLayer::Top, WindowLayer::Bottom] {
        let mut commits: Vec<(f64, f64, &str)> = windows
            .iter()
            .filter(|w| w.layer == layer)
            .map(|w| (w.commit_start, w.commit_end, w.id.as_str()))
            .collect();
        commits.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        for i in 1..commits.len() {
            if commits[i].0 < commits[i - 1].1 - 1e-9 {
                return Err(format!(
                    "overlapping commits in same layer: {} and {}",
                    commits[i - 1].2, commits[i].2
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_graph() -> OracleGraph {
        // 3 detectors in a line, two edges, one observable on middle edge
        OracleGraph {
            num_detectors: 3,
            num_observables: 1,
            detector_t: vec![0.0, 1.0, 2.0],
            edges: vec![
                OracleEdge {
                    endpoints: (Some(0), Some(1)),
                    observables: vec![],
                    t_mid: 0.5,
                },
                OracleEdge {
                    endpoints: (Some(1), Some(2)),
                    observables: vec![0],
                    t_mid: 1.5,
                },
                // boundary edge D2 -> virtual
                OracleEdge {
                    endpoints: (Some(2), None),
                    observables: vec![],
                    t_mid: 2.0,
                },
            ],
        }
    }

    #[test]
    fn residual_basic() {
        let g = tiny_graph();
        // syndrome at D0 and D1 -> edge 0 should annihilate them
        let committed = vec![0];
        let residual = residual_syndrome(&[0, 1], &committed, &g);
        assert_eq!(residual, Vec::<usize>::new());

        // syndrome at D0 alone, commit edge 0 -> flips D0 and D1, residual has D1 artificial
        let residual = residual_syndrome(&[0], &committed, &g);
        assert_eq!(residual, vec![1]);
    }

    #[test]
    fn observable_parity() {
        let g = tiny_graph();
        assert_eq!(observable_parity(&[0], &g), Vec::<usize>::new());
        assert_eq!(observable_parity(&[1], &g), vec![0]);
        assert_eq!(observable_parity(&[0, 1], &g), vec![0]);
    }

    #[test]
    fn commit_tentative_filters_buffer() {
        let g = tiny_graph();
        let windows = vec![
            WindowSpec {
                id: "A0".into(),
                layer: WindowLayer::Top,
                t_start: 0.0,
                t_end: 2.0,
                commit_start: 0.0,
                commit_end: 1.0,
                boundary: WindowBoundary::Open,
            },
            WindowSpec {
                id: "B0".into(),
                layer: WindowLayer::Bottom,
                t_start: 1.0,
                t_end: 3.0,
                commit_start: 1.0,
                commit_end: 3.0,
                boundary: WindowBoundary::Closed,
            },
        ];
        // Window A0 tentatively returns edges 0 (commit) and 1 (buffer)
        let mut tentative = BTreeMap::new();
        tentative.insert("A0".into(), vec![0, 1]);
        tentative.insert("B0".into(), vec![]);
        let result = commit_tentative(&tentative, &windows, &g, &[0, 1]);
        // Only edge 0 is inside A0 commit [0,1) (t_mid 0.5), edge 1 t_mid 1.5 is outside, discarded
        assert_eq!(result.committed_edges, vec![0]);
        assert_eq!(result.observable_parity, Vec::<usize>::new());
        // residual = input [0,1] xor boundary(edge0) = {}? edge0 flips 0 and1, so residual empty -> no artificial seam
        assert_eq!(result.residual_detectors, Vec::<usize>::new());
    }

    #[test]
    fn crossing_creates_artificial() {
        let g = tiny_graph();
        let windows = vec![WindowSpec {
            id: "A0".into(),
            layer: WindowLayer::Top,
            t_start: 0.0,
            t_end: 3.0,
            commit_start: 0.0,
            commit_end: 1.0,
            boundary: WindowBoundary::Open,
        }];
        // Syndrome at 0 and 2, matching edge path is 0-1-2 via edges 0 and1.
        // If inner decoder returns both edges, only edge0 is committed, leaving artificial at 1.
        let mut tentative = BTreeMap::new();
        tentative.insert("A0".into(), vec![0, 1]);
        let result = commit_tentative(&tentative, &windows, &g, &[0, 2]);
        assert_eq!(result.committed_edges, vec![0]);
        // residual = [0,2] xor {0,1} = {1,2} -> D1 is artificial seam, D2 remains
        assert_eq!(result.residual_detectors, vec![1, 2]);
        assert_eq!(result.observable_parity, Vec::<usize>::new());
    }

    #[test]
    fn validate_layers() {
        let ok = vec![
            WindowSpec {
                id: "A0".into(),
                layer: WindowLayer::Top,
                t_start: 0.0,
                t_end: 3.0,
                commit_start: 1.0,
                commit_end: 2.0,
                boundary: WindowBoundary::Open,
            },
            WindowSpec {
                id: "B0".into(),
                layer: WindowLayer::Bottom,
                t_start: 2.0,
                t_end: 3.0,
                commit_start: 2.0,
                commit_end: 3.0,
                boundary: WindowBoundary::Closed,
            },
        ];
        assert!(validate_window_layers(&ok).is_ok());
        let mut bad = ok.clone();
        bad[0].boundary = WindowBoundary::Closed;
        assert!(validate_window_layers(&bad).is_err());
    }
}
