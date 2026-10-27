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

fn initializer(input: &Initializer) -> Result<SolverInitializer, String> {
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

fn positions(input: &[Position]) -> Vec<VisualizePosition> {
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
        // Half-open time interval [first committed slice, next global slice).
        // A crossing edge's midpoint can therefore belong to the commit even
        // when its far endpoint is in the buffer. A physical boundary edge is
        // dated by its single real detector, never by the synthetic vertex.
        let commit_start = times[first_commit];
        let commit_end = times.get(last_commit + 1).copied().unwrap_or(f64::INFINITY);
        let mut commit_edges = BTreeSet::new();
        if plan.layer == 0 {
            for &eid in &cut.edge_map {
                let detectors = &graph.edge_metadata[eid].detectors;
                if !detectors.iter().all(|d| members.contains(d)) {
                    continue; // temporal virtual edge: tentative only
                }
                let mid = if detectors.len() == 1 {
                    detector_times[detectors[0]]
                } else {
                    detector_times[detectors[0]] * 0.5 + detector_times[detectors[1]] * 0.5
                };
                if (commit_start..commit_end).contains(&mid) {
                    commit_edges.insert(eid);
                }
            }
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
            commit_edges: commit_edges.into_iter().collect(),
        });
    }
    if let Some(missing) = (0..graph.num_detectors).find(|&d| detector_owner[d].is_none()) {
        return Err(format!(
            "detector {missing} is not owned by any window commit"
        ));
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
