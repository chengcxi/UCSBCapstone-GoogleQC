//! Explicit bridge between the validated toolchain graph, the cut window
//! graphs, and Micro Blossom's CPU parallel-window solver. No detector-ID plan
//! is silently interpreted as a time interval: shots require complete,
//! contiguous time slices and an unambiguous commit edge rule.

use std::collections::BTreeSet;

use fusion_blossom::util::SolverInitializer;
use fusion_blossom::visualize::VisualizePosition;
use micro_blossom::parallel_window::{EncodedWindow, ParallelWindowDecoder, WindowLayer};
use micro_blossom::resources::MicroBlossomSingle;
use serde::{Deserialize, Serialize};

use crate::window::{WindowAnnotation, WindowGraph};
use crate::{Graph, Initializer, Position};

/// Input shots use global detector IDs; optional observables are measured
/// flips, not predictions. The shape is identical for sampled and supplied
/// syndromes, so neither path can secretly fabricate a correction.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShotBatch {
    pub shots: Vec<ShotInput>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShotInput {
    pub defects: Vec<usize>,
    #[serde(default)]
    pub observables: Option<Vec<usize>>,
}

#[derive(Debug, Serialize)]
pub struct CorrectionBatch {
    pub schema_version: u32,
    pub num_detectors: usize,
    pub num_observables: usize,
    pub shots: Vec<CorrectionShot>,
}

#[derive(Debug, Serialize)]
pub struct CorrectionShot {
    pub defects: Vec<usize>,
    /// Indices in the GLOBAL `graph.json` initializer/edge_metadata order.
    pub committed_edges: Vec<usize>,
    pub predicted_observables: Vec<usize>,
    pub residual_syndrome: Vec<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual_observables: Option<Vec<usize>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logical_error: Option<bool>,
}

pub(crate) fn initializer(input: &Initializer) -> Result<SolverInitializer, String> {
    let edges = input
        .weighted_edges
        .iter()
        .map(|&(l, r, w)| {
            isize::try_from(w)
                .map(|weight| (l, r, weight))
                .map_err(|_| "edge weight does not fit the CPU solver".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(SolverInitializer::new(
        input.vertex_num,
        edges,
        input.virtual_vertices.clone(),
    ))
}

pub(crate) fn positions(input: &[Position]) -> Vec<VisualizePosition> {
    input
        .iter()
        .map(|pos| VisualizePosition::new(pos.i, pos.j, pos.t))
        .collect()
}

fn time_index(times: &[f64], t: f64) -> usize {
    // Every detector time came from this sorted list.
    times
        .binary_search_by(|candidate| candidate.total_cmp(&t))
        .unwrap()
}

/// Prepare a CPU decoder that solves precisely the window graphs which will
/// be published as `PREFIX.windows.json`. A window *annotation* alone is not
/// executable: reject missing time coordinates, spatially truncated slices,
/// noncontiguous commits and uncovered detector ownership first.
pub fn prepare_decoder(
    graph: &Graph,
    cut_windows: &[WindowGraph],
) -> Result<ParallelWindowDecoder, String> {
    if graph.window_annotations.is_null() {
        return Err("--shots/--sample-shots requires window annotations".into());
    }
    let annotations: Vec<WindowAnnotation> =
        serde_json::from_value(graph.window_annotations["windows"].clone())
            .map_err(|e| format!("invalid window annotations: {e}"))?;
    if cut_windows.len() != annotations.len() {
        return Err("window graph and annotation counts differ".into());
    }
    let detector_times: Vec<f64> = graph
        .detector_coordinates
        .iter()
        .enumerate()
        .map(|(d, c)| {
            c.get(2)
                .copied()
                .ok_or_else(|| format!("detector {d} needs a time coordinate for decoding"))
        })
        .collect::<Result<_, _>>()?;
    let mut times = detector_times.clone();
    times.sort_by(f64::total_cmp);
    times.dedup_by(|a, b| a.total_cmp(b).is_eq());
    let time_ids: Vec<usize> = detector_times
        .iter()
        .map(|&t| time_index(&times, t))
        .collect();
    let mut detector_owner = vec![None::<&str>; graph.num_detectors];
    let mut edge_owner = vec![None::<&str>; graph.edge_metadata.len()];
    let mut encoded = Vec::with_capacity(annotations.len());
    for (plan, cut) in annotations.iter().zip(cut_windows) {
        if let Some(&d) = plan
            .commit
            .iter()
            .chain(&plan.buffer)
            .find(|&&d| d >= graph.num_detectors)
        {
            return Err(format!("window {} detector {d} is out of range", plan.id));
        }
        for &d in &plan.commit {
            if let Some(other) = detector_owner[d] {
                return Err(format!(
                    "detector {d} has overlapping commits in windows {other} and {}",
                    plan.id
                ));
            }
            detector_owner[d] = Some(&plan.id);
        }
        if cut.window_id != plan.id
            || cut.layer != plan.layer
            || cut.temporal_boundary != plan.temporal_boundary
        {
            return Err(format!(
                "window {} graph does not match its annotation",
                plan.id
            ));
        }
        let commit: BTreeSet<_> = plan.commit.iter().copied().collect();
        let members: BTreeSet<_> = plan.commit.iter().chain(&plan.buffer).copied().collect();
        let member_times: BTreeSet<_> = members.iter().map(|&d| time_ids[d]).collect();
        let commit_times: BTreeSet<_> = commit.iter().map(|&d| time_ids[d]).collect();
        let first = *member_times.first().ok_or("empty window")?;
        let last = *member_times.last().ok_or("empty window")?;
        let first_commit = *commit_times.first().ok_or("empty commit")?;
        let last_commit = *commit_times.last().ok_or("empty commit")?;
        if member_times.len() != last - first + 1
            || commit_times.len() != last_commit - first_commit + 1
        {
            return Err(format!("window {} needs contiguous time slices", plan.id));
        }
        for (d, &tid) in time_ids.iter().enumerate() {
            if member_times.contains(&tid) && !members.contains(&d) {
                return Err(format!(
                    "window {} must include every detector in a time slice (missing {d})",
                    plan.id
                ));
            }
            if members.contains(&d) && commit_times.contains(&tid) != commit.contains(&d) {
                return Err(format!(
                    "window {} mixes commit and buffer in a time slice",
                    plan.id
                ));
            }
        }
        // Detector-ID commits must keep every selected edge incident to a
        // committed detector. This preserves the local solver's syndrome at
        // that detector on BOTH sides of the commit, leaving residuals only
        // in bottom regions. A midpoint interval starting at the first commit
        // slice incorrectly discards edges into the preceding buffer slice.
        let mut commit_edges = BTreeSet::new();
        if plan.layer == 0 {
            let local_edges: BTreeSet<_> = cut.edge_map.iter().copied().collect();
            for (eid, edge) in graph.edge_metadata.iter().enumerate() {
                let detectors = &edge.detectors;
                if !detectors.iter().any(|d| commit.contains(d)) {
                    continue; // buffer-only edges provide context, never a commit
                }
                if !detectors.iter().all(|d| members.contains(d)) {
                    return Err(format!(
                        "window {} needs buffer coverage for edge {eid} incident to its commit",
                        plan.id
                    ));
                }
                if !local_edges.contains(&eid) {
                    return Err(format!(
                        "window {} is missing owned edge {eid} from its cut graph",
                        plan.id
                    ));
                }
                commit_edges.insert(eid);
            }
        }
        // Detector coverage alone does not cover edges between two closed
        // bottom windows, or prevent an edge joining two top commits from
        // being owned twice. Reject both before processing any shots.
        let owned_edges: Vec<_> = if plan.layer == 0 {
            commit_edges.iter().copied().collect()
        } else {
            cut.edge_map.clone()
        };
        for eid in owned_edges {
            let owner = edge_owner
                .get_mut(eid)
                .ok_or_else(|| format!("window {} has an invalid edge map", plan.id))?;
            if let Some(other) = *owner {
                return Err(format!(
                    "edge {eid} has overlapping commits in windows {other} and {}",
                    plan.id
                ));
            }
            *owner = Some(&plan.id);
        }
        encoded.push(EncodedWindow {
            id: cut.window_id.clone(),
            layer: if plan.layer == 0 {
                WindowLayer::Top
            } else {
                WindowLayer::Bottom
            },
            initializer: initializer(&cut.initializer)?,
            positions: positions(&cut.positions),
            detector_map: cut.detector_map.clone(),
            edge_map: cut.edge_map.clone(),
            commit_detectors: plan.commit.clone(),
            commit_edges: commit_edges.into_iter().collect(),
        });
    }
    if let Some(missing) = (0..graph.num_detectors).find(|&d| detector_owner[d].is_none()) {
        return Err(format!(
            "detector {missing} is not owned by any window commit"
        ));
    }
    if let Some(missing) = edge_owner.iter().position(Option::is_none) {
        return Err(format!("edge {missing} is not owned by any window commit"));
    }
    let global_init = initializer(&graph.initializer)?;
    let global_graph = MicroBlossomSingle::new(&global_init, &positions(&graph.positions));
    let labels = graph
        .edge_metadata
        .iter()
        .map(|e| e.observables.clone())
        .collect();
    ParallelWindowDecoder::from_encoded(global_graph, encoded, labels, graph.num_observables)
}

/// Decode a nonempty batch; every correction is verified against the global
/// graph's independent XOR/parity algebra before returning any output.
pub fn decode_batch(
    graph: &Graph,
    decoder: &ParallelWindowDecoder,
    batch: ShotBatch,
) -> Result<CorrectionBatch, String> {
    if batch.shots.is_empty() {
        return Err("shots must contain at least one syndrome".into());
    }
    let mut shots = Vec::with_capacity(batch.shots.len());
    for (index, shot) in batch.shots.into_iter().enumerate() {
        let mut seen = BTreeSet::new();
        for &d in &shot.defects {
            if d >= graph.num_detectors || !seen.insert(d) {
                return Err(format!("shot {index}: invalid or duplicate detector {d}"));
            }
        }
        if let Some(actual) = &shot.observables {
            let mut seen = BTreeSet::new();
            for &obs in actual {
                if obs >= graph.num_observables || !seen.insert(obs) {
                    return Err(format!(
                        "shot {index}: invalid or duplicate observable {obs}"
                    ));
                }
            }
        }
        let result = decoder
            .decode_encoded_shot(&shot.defects)
            .map_err(|e| format!("shot {index}: {e}"))?;
        let parity = graph
            .evaluate_correction(&result.committed_edges, &shot.defects)
            .map_err(|e| format!("shot {index}: {e}"))?;
        if !parity.residual_syndrome.is_empty() {
            return Err(format!(
                "shot {index}: nonzero global residual {:?}",
                parity.residual_syndrome
            ));
        }
        let predicted: Vec<_> = result
            .logical_parity
            .iter()
            .enumerate()
            .filter_map(|(obs, &flipped)| flipped.then_some(obs))
            .collect();
        if parity.observables != predicted {
            return Err(format!(
                "shot {index}: CPU prediction differs from global edge parity"
            ));
        }
        let actual = shot.observables.map(|mut labels| {
            labels.sort_unstable();
            labels
        });
        let logical_error = actual.as_ref().map(|labels| labels != &predicted);
        shots.push(CorrectionShot {
            defects: shot.defects,
            committed_edges: result.committed_edges,
            predicted_observables: predicted,
            residual_syndrome: parity.residual_syndrome,
            actual_observables: actual,
            logical_error,
        });
    }
    Ok(CorrectionBatch {
        schema_version: 1,
        num_detectors: graph.num_detectors,
        num_observables: graph.num_observables,
        shots,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window::build_window_graphs;
    use crate::{Dem, convert};

    fn seam_graph() -> Graph {
        let ir: Dem = serde_json::from_value(serde_json::json!({
            "schema_version":1,"num_detectors":3,"num_observables":1,
            "detector_coordinates":[[0,0,0],[0,0,1],[0,0,2]],
            "errors":[
                {"probability":0.1,"components":[{"detectors":[0,1],"observables":[0]}]},
                {"probability":0.2,"components":[{"detectors":[1,2],"observables":[]}]},
                {"probability":0.05,"components":[{"detectors":[2],"observables":[]}]}
            ],
            "window_annotations":{"schema_version":1,"annotation_only":true,"windows":[
                {"id":"A0","layer":0,"commit":[0],"buffer":[1],"temporal_boundary":"open",
                 "commit_buffer_seams":[{"error":0,"component":0}],
                 "outer_crossings":[{"error":1,"component":0}]},
                {"id":"B0","layer":1,"commit":[1,2],"buffer":[],"temporal_boundary":"closed",
                 "commit_buffer_seams":[],"outer_crossings":[{"error":0,"component":0}]}
            ]}
        }))
        .unwrap();
        convert(ir, 100.0, false).unwrap()
    }

    fn alternating_graph() -> Graph {
        let ir: Dem = serde_json::from_value(serde_json::json!({
            "schema_version":1,"num_detectors":5,"num_observables":2,
            "detector_coordinates":[[0,0,0],[0,0,1],[0,0,2],[0,0,3],[0,0,4]],
            "errors":[
                {"probability":0.1,"components":[{"detectors":[0,1],"observables":[0]}]},
                {"probability":0.1,"components":[{"detectors":[1,2],"observables":[1]}]},
                {"probability":0.1,"components":[{"detectors":[2,3],"observables":[0,1]}]},
                {"probability":0.1,"components":[{"detectors":[3,4],"observables":[]}]}
            ],
            "window_annotations":{"schema_version":1,"annotation_only":true,"windows":[
                {"id":"A0","layer":0,"commit":[0],"buffer":[1],"temporal_boundary":"open",
                 "commit_buffer_seams":[{"error":0,"component":0}],
                 "outer_crossings":[{"error":1,"component":0}]},
                {"id":"B0","layer":1,"commit":[1,2],"buffer":[],"temporal_boundary":"closed",
                 "commit_buffer_seams":[],
                 "outer_crossings":[{"error":0,"component":0},{"error":2,"component":0}]},
                {"id":"A1","layer":0,"commit":[3,4],"buffer":[2],"temporal_boundary":"open",
                 "commit_buffer_seams":[{"error":2,"component":0}],
                 "outer_crossings":[{"error":1,"component":0}]}
            ]}
        }))
        .unwrap();
        convert(ir, 100.0, false).unwrap()
    }

    #[test]
    fn rejects_unowned_bottom_seam_and_shared_top_edge() {
        let mut graph = seam_graph();
        graph.window_annotations["windows"][1]["commit"] = serde_json::json!([1]);
        graph.window_annotations["windows"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "id":"B1","layer":1,"commit":[2],"buffer":[],"temporal_boundary":"closed"
            }));
        let cut = build_window_graphs(&graph).unwrap();
        let error = prepare_decoder(&graph, &cut).err().unwrap();
        assert!(error.contains("edge 1 is not owned"), "{error}");

        let mut graph = seam_graph();
        graph.window_annotations["windows"][1]["commit"] = serde_json::json!([2]);
        graph.window_annotations["windows"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "id":"A1","layer":0,"commit":[1],"buffer":[0,2],"temporal_boundary":"open"
            }));
        let cut = build_window_graphs(&graph).unwrap();
        let error = prepare_decoder(&graph, &cut).err().unwrap();
        assert!(error.contains("edge 0 has overlapping commits"), "{error}");
    }

    #[test]
    fn rejects_insufficient_buffer_and_missing_owned_edges() {
        let mut graph = alternating_graph();
        graph.window_annotations["windows"][2]["buffer"] = serde_json::json!([]);
        let cut = build_window_graphs(&graph).unwrap();
        let error = prepare_decoder(&graph, &cut).err().unwrap();
        assert!(
            error.contains("A1 needs buffer coverage for edge 2"),
            "{error}"
        );

        let graph = alternating_graph();
        let mut cut = build_window_graphs(&graph).unwrap();
        let index = cut[2].edge_map.iter().position(|&eid| eid == 2).unwrap();
        cut[2].edge_map.remove(index);
        cut[2].initializer.weighted_edges.remove(index);
        cut[2].edge_metadata.remove(index);
        let error = prepare_decoder(&graph, &cut).err().unwrap();
        assert!(error.contains("A1 is missing owned edge 2"), "{error}");
    }

    #[test]
    fn exhaustive_line_faults_survive_both_seams_and_time_reversal() {
        for times in [
            [0.0, 1.0, 2.0, 3.0, 4.0],
            [4.0, 3.0, 2.0, 1.0, 0.0],
            [-10.0, -9.9, 0.0, 100.0, 1000.0],
        ] {
            let mut graph = alternating_graph();
            for ((coords, pos), t) in graph
                .detector_coordinates
                .iter_mut()
                .zip(&mut graph.positions)
                .zip(times)
            {
                coords[2] = t;
                pos.t = t;
            }
            let cut = build_window_graphs(&graph).unwrap();
            let decoder = prepare_decoder(&graph, &cut).unwrap();
            // A line is a tree: every possible fault subset has a unique
            // correction, including chains spanning both seams and the ends.
            for mask in 0..1 << graph.edge_metadata.len() {
                let edges: Vec<_> = (0..graph.edge_metadata.len())
                    .filter(|eid| mask & (1 << eid) != 0)
                    .collect();
                let expected = graph.evaluate_correction(&edges, &[]).unwrap();
                let batch = ShotBatch {
                    shots: vec![ShotInput {
                        defects: expected.detectors,
                        observables: Some(expected.observables),
                    }],
                };
                let decoded = decode_batch(&graph, &decoder, batch)
                    .unwrap_or_else(|e| panic!("times {times:?}, fault mask {mask}: {e}"));
                assert_eq!(decoded.shots[0].committed_edges, edges);
                assert!(decoded.shots[0].residual_syndrome.is_empty());
                assert_eq!(decoded.shots[0].logical_error, Some(false));
            }
        }
    }

    #[test]
    fn decodes_both_commit_seams_and_artificial_defects() {
        let graph = alternating_graph();
        let cut = build_window_graphs(&graph).unwrap();
        let decoder = prepare_decoder(&graph, &cut).unwrap();
        let batch: ShotBatch = serde_json::from_value(serde_json::json!({"shots":[
            {"defects":[2,3], "observables":[0,1]},
            {"defects":[0,1], "observables":[0]},
            {"defects":[1,3], "observables":[0]},
            {"defects":[0,4], "observables":[]},
            {"defects":[3,4], "observables":[]}
        ]}))
        .unwrap();
        let decoded = decode_batch(&graph, &decoder, batch).unwrap();
        let expected = [vec![2], vec![0], vec![1, 2], vec![0, 1, 2, 3], vec![3]];
        for (shot, edges) in decoded.shots.iter().zip(expected) {
            assert_eq!(shot.committed_edges, edges);
            assert!(shot.residual_syndrome.is_empty());
            assert_eq!(shot.logical_error, Some(false));
        }
        // The right top commit toggles D2, creating B0's artificial event.
        let top_edges: Vec<_> = decoded.shots[2]
            .committed_edges
            .iter()
            .copied()
            .filter(|&eid| graph.edge_metadata[eid].detectors.contains(&3))
            .collect();
        assert_eq!(top_edges, vec![2]);
        assert_eq!(
            graph
                .evaluate_correction(&top_edges, &[1, 3])
                .unwrap()
                .residual_syndrome,
            vec![1, 2]
        );
    }

    #[test]
    fn decoded_seam_and_boundary_have_zero_residual() {
        let graph = seam_graph();
        let cut = build_window_graphs(&graph).unwrap();
        let decoder = prepare_decoder(&graph, &cut).unwrap();
        let batch: ShotBatch = serde_json::from_value(serde_json::json!({"shots":[
            {"defects":[0,2], "observables":[0]},
            {"defects":[0,1], "observables":[0]},
            {"defects":[2], "observables":[]},
            {"defects":[], "observables":[]}
        ]}))
        .unwrap();
        let decoded = decode_batch(&graph, &decoder, batch).unwrap();
        assert_eq!(decoded.shots[0].committed_edges, vec![0, 1]);
        assert_eq!(decoded.shots[0].predicted_observables, vec![0]);
        assert!(
            decoded
                .shots
                .iter()
                .all(|shot| shot.residual_syndrome.is_empty())
        );
        assert!(
            decoded
                .shots
                .iter()
                .all(|shot| shot.logical_error == Some(false))
        );
    }

    #[test]
    fn incident_ownership_both_seams_and_full_graph() {
        // Exhaust every syndrome, including both experiment endpoints.
        for commit in [vec![1], vec![0], vec![2], vec![0, 1, 2]] {
            let mut graph = seam_graph();
            let others: Vec<_> = (0..3).filter(|d| !commit.contains(d)).collect();
            graph.window_annotations["windows"][0]["commit"] = serde_json::json!(commit);
            graph.window_annotations["windows"][0]["buffer"] = serde_json::json!(others);
            if others.is_empty() {
                graph.window_annotations["windows"]
                    .as_array_mut()
                    .unwrap()
                    .truncate(1);
            } else {
                graph.window_annotations["windows"]
                    .as_array_mut()
                    .unwrap()
                    .truncate(1);
                let groups = if commit == vec![1] {
                    vec![vec![0], vec![2]]
                } else {
                    vec![others.clone()]
                };
                for (i, group) in groups.iter().enumerate() {
                    graph.window_annotations["windows"].as_array_mut().unwrap().push(serde_json::json!({
                        "id": format!("B{i}"), "layer":1,"commit":group,"buffer":[],
                        "temporal_boundary":"closed", "commit_buffer_seams":[], "outer_crossings":[]
                    }));
                }
            }
            let cut = build_window_graphs(&graph).unwrap();
            let decoder = prepare_decoder(&graph, &cut).unwrap();
            for mask in 0..8 {
                let defects: Vec<_> = (0..3).filter(|d| mask & (1 << d) != 0).collect();
                let result = decoder.decode_encoded_shot(&defects).unwrap();
                assert!(
                    graph
                        .evaluate_correction(&result.committed_edges, &defects)
                        .unwrap()
                        .residual_syndrome
                        .is_empty()
                );
                if commit == vec![0, 1, 2] {
                    assert_eq!(result.logical_parity, vec![mask & 1 != 0]);
                }
                if commit == vec![1] && mask == 3 {
                    assert_eq!(result.committed_edges, vec![0]);
                }
                if commit == vec![1] && mask == 6 {
                    assert_eq!(result.committed_edges, vec![1]);
                }
            }
        }
    }

    #[test]
    fn rejects_two_top_windows_claiming_one_edge() {
        let mut graph = seam_graph();
        graph.window_annotations["windows"][1] = serde_json::json!({
            "id":"A1", "layer":0, "commit":[1,2], "buffer":[0],
            "temporal_boundary":"open", "commit_buffer_seams":[], "outer_crossings":[]
        });
        let cut = build_window_graphs(&graph).unwrap();
        assert!(
            prepare_decoder(&graph, &cut)
                .err()
                .unwrap()
                .contains("edge 0 has overlapping commits")
        );
    }

    #[test]
    fn rejects_insufficient_context() {
        let mut graph = seam_graph();
        graph.window_annotations["windows"][0]["commit"] = serde_json::json!([1]);
        graph.window_annotations["windows"][0]["buffer"] = serde_json::json!([2]);
        let cut = build_window_graphs(&graph).unwrap();
        assert!(
            prepare_decoder(&graph, &cut)
                .err()
                .unwrap()
                .contains("needs buffer coverage")
        );
    }

    #[test]
    fn refuses_unowned_or_non_temporal_plans() {
        let mut graph = seam_graph();
        graph.window_annotations["windows"][1]["commit"] = serde_json::json!([1]);
        let cut = build_window_graphs(&graph).unwrap();
        assert!(
            prepare_decoder(&graph, &cut)
                .err()
                .unwrap()
                .contains("detector 2 is not owned")
        );
        let mut graph = seam_graph();
        graph.detector_coordinates[1].truncate(2);
        let cut = build_window_graphs(&graph).unwrap();
        assert!(
            prepare_decoder(&graph, &cut)
                .err()
                .unwrap()
                .contains("needs a time coordinate")
        );
    }

    #[test]
    fn rejects_ambiguous_ownership_and_mismatched_cut_graphs() {
        let mut graph = seam_graph();
        graph.window_annotations["windows"][1]["commit"] = serde_json::json!([0, 1, 2]);
        let cut = build_window_graphs(&graph).unwrap();
        assert!(
            prepare_decoder(&graph, &cut)
                .err()
                .unwrap()
                .contains("detector 0 has overlapping commits")
        );

        let graph = seam_graph();
        let mut cut = build_window_graphs(&graph).unwrap();
        cut[0].initializer.weighted_edges[0].0 = 1;
        assert!(
            prepare_decoder(&graph, &cut)
                .err()
                .unwrap()
                .contains("mismatched endpoints")
        );
    }
}
