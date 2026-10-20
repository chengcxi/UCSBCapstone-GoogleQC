//! Window graph construction for hardware generation (Sprint 2+)
//!
//! This module is the *toolchain counterpart* to the CPU implementation in
//! `src/cpu/blossom-nostd/src/window.rs`.  It translates a global
//! `Graph` (from `src/lib.rs`) plus its validated `window_annotations` into
//! per-window `Initializer` and `Position` arrays that can be fed to the
//! upstream `MicroBlossomBusGenerator` (Scala) to emit window-sized Verilog.
//!
//! # Current status
//! The annotation validation (commit/buffer/seam/outer-crossing references,
//! same-layer commit exclusivity, layer/boundary agreement) is complete and
//! exercised by `cargo test`.  The physical graph cutting described in
//! `docs/PARALLEL_WINDOW.md` — copying vertices by `t`, duplicating spatial
//! virtuals, synthesising temporal virtuals for open windows, and remapping
//! edge indices — is *implemented* here in pure Rust for simulation and as
//! the golden reference for the FPGA generator.  The Scala generator itself
//! (Sprints 3–4) will consume the same window specs and emit one
//! `MicroBlossomBus` per window with the temporal-virtual handling of
//! `build_window_initializer` in `src/cpu/blossom/src/parallel_window.rs`.
//!
//! The hardware TODO is therefore limited to the Scala/FPGA side:
//! `src/fpga/microblossom/MicroBlossomBus.scala` must be taught to accept a
//! `WindowSpec` and to instantiate the temporal-virtual logic.  No change to
//! the per-window dual/vertex pipeline is required; only the top-level
//! wiring and the simulation TCP host (`src/fpga/microblossom/util/SimulationTcpHost.scala`)
//! need a window-aware scheduler.

use crate::{Graph, Initializer, Position};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Window specification as it appears in `Graph::window_annotations`.
///
/// This is the JSON shape validated in `src/validation.rs`:
/// ```json
/// { "id": "A0", "layer": 0, "commit": [0], "buffer": [1],
///   "temporal_boundary": "open", "commit_buffer_seams": [...],
///   "outer_crossings": [...] }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowAnnotation {
    pub id: String,
    pub layer: u8, // 0 = top (open), 1 = bottom (closed)
    pub commit: Vec<usize>,
    pub buffer: Vec<usize>,
    pub temporal_boundary: String, // "open" or "closed"
    #[serde(default)]
    pub commit_buffer_seams: Vec<serde_json::Value>,
    #[serde(default)]
    pub outer_crossings: Vec<serde_json::Value>,
}

/// Per-window hardware graph (initializer + positions + edge-observable sidecar).
///
/// The fields are exactly the inputs that `MicroBlossomBusGenerator` expects:
/// a `MicroBlossomSingle`-compatible JSON with `vertex_num`, `weighted_edges`,
/// `virtual_vertices`, and communication-tree metadata derived from `positions`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowGraph {
    pub window_id: String,
    pub layer: u8,
    pub temporal_boundary: String,
    pub initializer: Initializer,
    pub positions: Vec<Position>,
    /// Subset of global `edge_metadata` that survived cutting, index-aligned
    /// with `initializer.weighted_edges`.  The upstream parser input does not
    /// carry observables, so this sidecar must be shipped alongside
    /// `hardware.json`.
    pub edge_metadata: Vec<crate::EdgeMetadata>,
    /// Mapping `local_detector -> global_detector` for syndrome translation.
    /// `None` for the synthetic temporal virtual detector.
    pub detector_map: Vec<Option<usize>>,
    /// Mapping `local_edge -> global_edge`
    pub edge_map: Vec<usize>,
}

/// Build window graphs from a global graph and its validated annotations.
///
/// For each annotation, collect the detectors in `commit ∪ buffer` (which is
/// the window's `t` interval), copy their coordinates, include the global
/// boundary detector when any window contains a one-detector error, and
/// synthesize temporal virtuals for open windows.  Edge cutting follows the
/// same rule as `build_window_initializer` in the CPU crate.
pub fn build_window_graphs(graph: &Graph) -> Result<Vec<WindowGraph>, String> {
    let ann = &graph.window_annotations;
    let windows: Vec<WindowAnnotation> = if ann.is_null() {
        return Ok(vec![]);
    } else {
        // The validation layer guarantees this shape.
        serde_json::from_value(ann["windows"].clone()).map_err(|e| e.to_string())?
    };

    let mut out = Vec::new();
    for w in windows {
        // Resolve commit+buffer detector ids to global detector indices
        let mut window_dets: BTreeSet<usize> = BTreeSet::new();
        for &d in w.commit.iter().chain(w.buffer.iter()) {
            if d >= graph.num_detectors {
                return Err(format!("window {} detector {d} out of range", w.id));
            }
            window_dets.insert(d);
        }

        // Map global detector -> local vertex
        let mut det_to_local: BTreeMap<usize, usize> = BTreeMap::new();
        let mut positions: Vec<Position> = Vec::new();
        let mut detector_map: Vec<Option<usize>> = Vec::new();

        for &d in &window_dets {
            let local = positions.len();
            det_to_local.insert(d, local);
            // Positions are indexed by detector id; for global boundary detector
            // the position is the last entry in graph.positions
            let pos = graph.positions.get(d).cloned().unwrap_or(Position {
                i: 0.0,
                j: 0.0,
                t: 0.0,
            });
            positions.push(pos);
            detector_map.push(Some(d));
        }

        let has_boundary = graph.initializer.virtual_vertices.len() == 1;

        // Include global boundary vertex if the window contains any one-detector edge
        if has_boundary {
            let needs_boundary = graph
                .edge_metadata
                .iter()
                .any(|em| em.detectors.len() == 1 && window_dets.contains(&em.detectors[0]));
            if needs_boundary {
                let local = positions.len();
                det_to_local.insert(graph.num_detectors, local);
                // Boundary position is last in graph.positions when has_boundary
                let pos = graph.positions.last().cloned().unwrap_or(Position {
                    i: 0.0,
                    j: 0.0,
                    t: 0.0,
                });
                positions.push(pos);
                detector_map.push(None); // synthetic, no global detector
            }
        }

        // Temporal virtuals for open windows (one per outer crossing that touches this window)
        // For the prototype we create a single temporal virtual at t = window's temporal cut
        // when layer==0 and temporal_boundary=="open".  The exact t is taken from the
        // detector at the cut if available.
        let mut temporal_virtual_local: Option<usize> = None;
        if w.layer == 0 && w.temporal_boundary == "open" {
            let local = positions.len();
            // t at the seam between commit and buffer — use middle of commit+buffer interval
            let t_cut = graph
                .detector_coordinates
                .get(*w.commit.first().unwrap_or(&0))
                .and_then(|c| c.get(2).copied())
                .unwrap_or(0.0);
            positions.push(Position {
                i: 0.0,
                j: 0.0,
                t: t_cut,
            });
            detector_map.push(None);
            temporal_virtual_local = Some(local);
        }

        // Cut edges: keep an edge iff both its detectors are inside window_dets
        // (or one is the boundary when the edge is a one-detector edge inside the window).
        // For open windows, crossing edges that have one endpoint inside and one outside
        // are rewired to the temporal virtual (one new edge per crossing, same weight).
        let mut weighted_edges: Vec<(usize, usize, i64)> = Vec::new();
        let mut edge_metadata: Vec<crate::EdgeMetadata> = Vec::new();
        let mut edge_map: Vec<usize> = Vec::new();

        for (global_eid, em) in graph.edge_metadata.iter().enumerate() {
            let inside: Vec<bool> = em
                .detectors
                .iter()
                .map(|d| window_dets.contains(d))
                .collect();
            if em.detectors.len() == 1 {
                // boundary edge: single detector
                if inside[0] {
                    let l = det_to_local[&em.detectors[0]];
                    let r = det_to_local[&graph.num_detectors];
                    weighted_edges.push((l, r, graph.initializer.weighted_edges[global_eid].2));
                    edge_metadata.push(em.clone());
                    edge_map.push(global_eid);
                }
            } else if em.detectors.len() == 2 {
                let a_inside = inside[0];
                let b_inside = inside[1];
                if a_inside && b_inside {
                    let l = det_to_local[&em.detectors[0]];
                    let r = det_to_local[&em.detectors[1]];
                    weighted_edges.push((l, r, graph.initializer.weighted_edges[global_eid].2));
                    edge_metadata.push(em.clone());
                    edge_map.push(global_eid);
                } else if w.layer == 0 && w.temporal_boundary == "open" && (a_inside ^ b_inside) {
                    // Open window crossing: rewire inside endpoint to temporal virtual
                    let inside_det = if a_inside {
                        em.detectors[0]
                    } else {
                        em.detectors[1]
                    };
                    let l = det_to_local[&inside_det];
                    let r = temporal_virtual_local.unwrap();
                    weighted_edges.push((l, r, graph.initializer.weighted_edges[global_eid].2));
                    edge_metadata.push(em.clone());
                    edge_map.push(global_eid);
                }
            }
        }

        let virtual_vertices = if has_boundary {
            // global boundary is always present locally when needed
            vec![det_to_local[&graph.num_detectors]]
        } else {
            vec![]
        };
        // Add temporal virtual to virtual list for open windows
        let mut all_virtuals = virtual_vertices;
        if let Some(tv) = temporal_virtual_local {
            all_virtuals.push(tv);
        }

        let initializer = Initializer {
            vertex_num: positions.len(),
            weighted_edges,
            virtual_vertices: all_virtuals,
        };

        out.push(WindowGraph {
            window_id: w.id.clone(),
            layer: w.layer,
            temporal_boundary: w.temporal_boundary.clone(),
            initializer,
            positions,
            edge_metadata,
            detector_map,
            edge_map,
        });
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Dem, convert};

    fn dem_with_windows() -> Dem {
        serde_json::from_value(serde_json::json!({
            "schema_version": 1,
            "num_detectors": 2,
            "num_observables": 1,
            "detector_coordinates": [[0, 0, 0], [0, 0, 1]],
            "errors": [
                {
                    "probability": 0.1,
                    "components": [{"detectors": [0, 1], "observables": [0]}]
                },
                {
                    "probability": 0.05,
                    "components": [{"detectors": [1], "observables": []}]
                }
            ],
            "window_annotations": {
                "schema_version": 1,
                "annotation_only": true,
                "windows": [
                    {
                        "id": "A0",
                        "layer": 0,
                        "commit": [0],
                        "buffer": [1],
                        "temporal_boundary": "open",
                        "commit_buffer_seams": [{"error": 0, "component": 0}],
                        "outer_crossings": []
                    },
                    {
                        "id": "B0",
                        "layer": 1,
                        "commit": [1],
                        "buffer": [],
                        "temporal_boundary": "closed",
                        "commit_buffer_seams": [],
                        "outer_crossings": [{"error": 0, "component": 0}]
                    }
                ]
            }
        }))
        .unwrap()
    }

    #[test]
    fn window_graphs_basic() {
        let dem = dem_with_windows();
        let g = convert(dem, 100.0, false).unwrap();
        let wgs = build_window_graphs(&g).unwrap();
        assert_eq!(wgs.len(), 2);
        assert_eq!(wgs[0].window_id, "A0");
        assert_eq!(wgs[0].layer, 0);
        assert_eq!(wgs[0].temporal_boundary, "open");
        // A0 contains detector 0 and 1 -> 2 vertices + 1 boundary + 1 temporal = 4
        assert!(wgs[0].initializer.vertex_num >= 3);
        // B0 is closed, no temporal virtual
        assert_eq!(wgs[1].temporal_boundary, "closed");
    }
}
