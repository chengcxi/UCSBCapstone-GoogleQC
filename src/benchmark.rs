//! Reproducible, CPU-only comparison of the original single-graph Micro
//! Blossom solver and the current sequential two-layer window decoder.
//!
//! Both paths use `SolverEmbeddedComb` with offloading and layer fusion
//! disabled. A timed sample includes per-shot solver construction, solving and
//! subgraph extraction; graph/IR preparation, Stim, parity checking and JSON
//! output are outside the timer. No FPGA execution or projected speedup.

use std::collections::BTreeSet;
use std::time::Instant;

use fusion_blossom::mwpm_solver::PrimalDualSolver;
use fusion_blossom::util::{SolverInitializer, SyndromePattern};
use fusion_blossom::visualize::VisualizePosition;
use micro_blossom::mwpm_solver::SolverEmbeddedComb;
use micro_blossom::parallel_window::ParallelWindowDecoder;
use micro_blossom::resources::MicroBlossomSingle;
use serde::Serialize;
use serde_json::json;

use crate::Graph;
use crate::decoding::{ShotBatch, ShotInput, initializer, positions};

#[derive(Debug, Serialize)]
pub struct Statistics {
    pub median_ns: f64,
    pub p95_ns: u64,
    pub mean_ns: f64,
    pub min_ns: u64,
    pub max_ns: u64,
}

/// Exact logical-observable prediction matches among shots with actual flips.
/// Each input shot counts once, regardless of warmup or measured pass count.
#[derive(Debug, Serialize)]
pub struct AccuracyStatistics {
    pub correct: usize,
    pub total: usize,
    /// None if no shot in this population has ground-truth observables.
    pub fraction: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct AccuracyBreakdown {
    pub all_shots: AccuracyStatistics,
    pub nonempty_shots: AccuracyStatistics,
}

#[derive(Debug, Serialize)]
pub struct MethodReport {
    pub id: &'static str,
    pub execution: &'static str,
    pub all_shots: Statistics,
    /// None only when the supplied batch consists entirely of empty syndromes.
    pub nonempty_shots: Option<Statistics>,
    /// One timing sample per input shot per measured pass, ordered pass then shot.
    pub samples_ns: Vec<u64>,
    pub shots_with_truth: usize,
    pub logical_errors: usize,
    /// Additive schema-v1 field; older reports may lack it. Logical accuracy
    /// is distinct from the global syndrome parity check (which must pass).
    pub logical_accuracy: AccuracyBreakdown,
}

#[derive(Debug, Serialize)]
pub struct ShotComparison {
    pub index: usize,
    pub defects: Vec<usize>,
    pub actual_observables: Option<Vec<usize>>,
    pub baseline_edges: Vec<usize>,
    pub window_edges: Vec<usize>,
    pub baseline_observables: Vec<usize>,
    pub window_observables: Vec<usize>,
    /// Different valid MWPM corrections need not have identical edge IDs.
    pub same_prediction: bool,
}

#[derive(Debug, Serialize)]
pub struct BenchmarkSettings {
    pub warmup_passes: usize,
    pub measured_passes: usize,
    pub shot_count: usize,
    pub nonempty_shot_count: usize,
    pub samples_per_method: usize,
    pub timing_scope: &'static str,
    pub order: &'static str,
}

#[derive(Debug, Serialize)]
pub struct BenchmarkReport {
    pub schema_version: u32,
    pub kind: &'static str,
    pub provenance: serde_json::Value,
    pub graph: GraphSize,
    pub host: Host,
    pub settings: BenchmarkSettings,
    pub baseline: MethodReport,
    pub windowed: MethodReport,
    pub prediction_disagreements: usize,
    pub shots: Vec<ShotComparison>,
    pub fpga: &'static str,
}

#[derive(Debug, Serialize)]
pub struct GraphSize {
    pub detectors: usize,
    pub edges: usize,
    pub windows: usize,
}

#[derive(Debug, Serialize)]
pub struct Host {
    pub os: &'static str,
    pub arch: &'static str,
    pub logical_cpus: Option<usize>,
    pub profile: &'static str,
}

fn statistics(values: &[u64]) -> Statistics {
    // Nonempty by construction: a benchmark requires at least one pass/shot.
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let n = sorted.len();
    Statistics {
        median_ns: (sorted[(n - 1) / 2] as f64 + sorted[n / 2] as f64) / 2.0,
        p95_ns: sorted[(95 * n).div_ceil(100) - 1],
        mean_ns: sorted.iter().map(|&value| value as f64).sum::<f64>() / n as f64,
        min_ns: sorted[0],
        max_ns: sorted[n - 1],
    }
}

fn logical_accuracy(
    outcomes: &[ShotComparison],
    nonempty_only: bool,
    baseline: bool,
) -> AccuracyStatistics {
    let mut correct = 0;
    let mut total = 0;
    for shot in outcomes {
        if nonempty_only && shot.defects.is_empty() {
            continue;
        }
        if let Some(actual) = &shot.actual_observables {
            total += 1;
            let predicted = if baseline {
                &shot.baseline_observables
            } else {
                &shot.window_observables
            };
            if predicted == actual {
                correct += 1;
            }
        }
    }
    AccuracyStatistics {
        correct,
        total,
        fraction: (total > 0).then(|| correct as f64 / total as f64),
    }
}

fn method_report(
    id: &'static str,
    samples: Vec<u64>,
    nonempty: Vec<u64>,
    logical_accuracy: AccuracyBreakdown,
) -> MethodReport {
    MethodReport {
        id,
        execution: "sequential_cpu",
        all_shots: statistics(&samples),
        nonempty_shots: (!nonempty.is_empty()).then(|| statistics(&nonempty)),
        samples_ns: samples,
        shots_with_truth: logical_accuracy.all_shots.total,
        logical_errors: logical_accuracy.all_shots.total - logical_accuracy.all_shots.correct,
        logical_accuracy,
    }
}

fn check_input(graph: &Graph, shot: &ShotInput, index: usize) -> Result<(), String> {
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
    // As for the window solver, reject an odd, boundary-free component rather
    // than allowing the global inner solver to panic on an impossible shot.
    let mut adjacency = vec![Vec::new(); graph.initializer.vertex_num];
    for &(l, r, _) in &graph.initializer.weighted_edges {
        adjacency[l].push(r);
        adjacency[r].push(l);
    }
    let virtuals: BTreeSet<_> = graph.initializer.virtual_vertices.iter().copied().collect();
    let defect_set: BTreeSet<_> = shot.defects.iter().copied().collect();
    let mut visited = vec![false; adjacency.len()];
    for start in 0..adjacency.len() {
        if visited[start] {
            continue;
        }
        let mut stack = vec![start];
        visited[start] = true;
        let mut odd = false;
        let mut boundary = false;
        while let Some(v) = stack.pop() {
            odd ^= defect_set.contains(&v);
            boundary |= virtuals.contains(&v);
            for &next in &adjacency[v] {
                if !visited[next] {
                    visited[next] = true;
                    stack.push(next);
                }
            }
        }
        if odd && !boundary {
            return Err(format!(
                "shot {index}: unmatchable global syndrome component"
            ));
        }
    }
    Ok(())
}

fn solve_global(
    init: &SolverInitializer,
    pos: &[VisualizePosition],
    defects: &[usize],
) -> Vec<usize> {
    if defects.is_empty() {
        return Vec::new();
    }
    // The exact same CPU solver configuration as solve_encoded_window in the
    // existing Micro Blossom window implementation. Both rebuild per shot.
    let mut solver = SolverEmbeddedComb::new(
        MicroBlossomSingle::new(init, pos),
        json!({"dual": {"sim_config": {
            "support_offloading": false, "support_layer_fusion": false
        }}}),
    );
    solver.solve_visualizer(&SyndromePattern::new_vertices(defects.to_vec()), None);
    solver.subgraph_visualizer(None)
}

fn verify(
    graph: &Graph,
    shot: &ShotInput,
    edges: &[usize],
    index: usize,
    name: &str,
) -> Result<Vec<usize>, String> {
    let parity = graph.evaluate_correction(edges, &shot.defects)?;
    if !parity.residual_syndrome.is_empty() {
        return Err(format!(
            "shot {index}: {name} left a nonzero global residual {:?}",
            parity.residual_syndrome
        ));
    }
    Ok(parity.observables)
}

fn elapsed_ns(start: Instant) -> Result<u64, String> {
    u64::try_from(start.elapsed().as_nanos())
        .map_err(|_| "benchmark duration exceeds u64 nanoseconds".into())
}

/// Benchmark *the same input shots* under both CPU paths. The order alternates
/// per shot and pass to reduce systematic warm-cache/order bias. Each result
/// is checked outside the timed region; report is emitted only if all pass.
pub fn compare_cpu_decoders(
    graph: &Graph,
    windowed: &ParallelWindowDecoder,
    batch: &ShotBatch,
    warmup_passes: usize,
    measured_passes: usize,
) -> Result<BenchmarkReport, String> {
    let shot_count = batch.shots.len();
    if shot_count == 0 || shot_count > 1_000 {
        return Err("benchmark needs between 1 and 1000 shots".into());
    }
    if warmup_passes > 20 || !(1..=1_000).contains(&measured_passes) {
        return Err("warmup must be 0..20 and measured passes must be 1..1000".into());
    }
    let sample_count = shot_count
        .checked_mul(measured_passes)
        .ok_or("benchmark size overflow")?;
    if sample_count > 50_000 {
        return Err("benchmark is limited to 50000 measured shots per method".into());
    }
    for (index, shot) in batch.shots.iter().enumerate() {
        check_input(graph, shot, index)?;
    }
    let nonempty_count = batch
        .shots
        .iter()
        .filter(|shot| !shot.defects.is_empty())
        .count();
    let init = initializer(&graph.initializer)?;
    let pos = positions(&graph.positions);
    let mut baseline_samples = Vec::with_capacity(sample_count);
    let mut window_samples = Vec::with_capacity(sample_count);
    let mut baseline_nonempty = Vec::with_capacity(nonempty_count * measured_passes);
    let mut window_nonempty = Vec::with_capacity(nonempty_count * measured_passes);
    let mut outcomes: Vec<ShotComparison> = Vec::with_capacity(shot_count);

    for pass in 0..(warmup_passes + measured_passes) {
        for (index, shot) in batch.shots.iter().enumerate() {
            let measure_global = || {
                let start = Instant::now();
                let edges = solve_global(&init, &pos, &shot.defects);
                elapsed_ns(start).map(|elapsed| (edges, elapsed))
            };
            let measure_windows = || {
                let start = Instant::now();
                let result = windowed
                    .decode_encoded_shot(&shot.defects)
                    .map_err(|error| format!("shot {index}: {error}"))?;
                elapsed_ns(start).map(|elapsed| (result, elapsed))
            };
            let (baseline, baseline_ns, windows, window_ns) = if (pass + index) % 2 == 0 {
                let (baseline, baseline_ns) = measure_global()?;
                let (windows, window_ns) = measure_windows()?;
                (baseline, baseline_ns, windows, window_ns)
            } else {
                let (windows, window_ns) = measure_windows()?;
                let (baseline, baseline_ns) = measure_global()?;
                (baseline, baseline_ns, windows, window_ns)
            };
            let baseline_observables = verify(graph, shot, &baseline, index, "global CPU")?;
            let window_observables =
                verify(graph, shot, &windows.committed_edges, index, "window CPU")?;
            let window_prediction: Vec<_> = windows
                .logical_parity
                .iter()
                .enumerate()
                .filter_map(|(obs, &flipped)| flipped.then_some(obs))
                .collect();
            if window_observables != window_prediction {
                return Err(format!(
                    "shot {index}: window CPU prediction differs from global edge parity"
                ));
            }
            if pass == 0 {
                let mut actual = shot.observables.clone();
                if let Some(ref mut values) = actual {
                    values.sort_unstable();
                }
                outcomes.push(ShotComparison {
                    index,
                    defects: shot.defects.clone(),
                    actual_observables: actual,
                    baseline_edges: baseline.clone(),
                    window_edges: windows.committed_edges.clone(),
                    same_prediction: baseline_observables == window_observables,
                    baseline_observables: baseline_observables.clone(),
                    window_observables: window_observables.clone(),
                });
            } else if outcomes[index].baseline_observables != baseline_observables
                || outcomes[index].window_observables != window_observables
            {
                return Err(format!(
                    "shot {index}: decoder results varied between repetitions"
                ));
            }
            if pass >= warmup_passes {
                baseline_samples.push(baseline_ns);
                window_samples.push(window_ns);
                if !shot.defects.is_empty() {
                    baseline_nonempty.push(baseline_ns);
                    window_nonempty.push(window_ns);
                }
            }
        }
    }
    let baseline_accuracy = AccuracyBreakdown {
        all_shots: logical_accuracy(&outcomes, false, true),
        nonempty_shots: logical_accuracy(&outcomes, true, true),
    };
    let window_accuracy = AccuracyBreakdown {
        all_shots: logical_accuracy(&outcomes, false, false),
        nonempty_shots: logical_accuracy(&outcomes, true, false),
    };
    let prediction_disagreements = outcomes.iter().filter(|shot| !shot.same_prediction).count();
    Ok(BenchmarkReport {
        schema_version: 1,
        kind: "cpu_decoder_comparison",
        provenance: graph.provenance.clone(),
        graph: GraphSize {
            detectors: graph.num_detectors,
            edges: graph.edge_metadata.len(),
            windows: graph.window_annotations["windows"]
                .as_array()
                .map_or(0, Vec::len),
        },
        host: Host {
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            logical_cpus: std::thread::available_parallelism()
                .ok()
                .map(std::num::NonZero::get),
            profile: if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            },
        },
        settings: BenchmarkSettings {
            warmup_passes,
            measured_passes,
            shot_count,
            nonempty_shot_count: nonempty_count,
            samples_per_method: sample_count,
            timing_scope: concat!(
                "per-shot decoder calls; includes solver construction, solve, subgraph extraction, ",
                "and window-local routing/stitching; excludes Stim, graph preparation, ",
                "external parity verification and I/O"
            ),
            order: "alternated per shot and pass; no concurrent execution",
        },
        baseline: method_report(
            "micro_blossom_global_cpu",
            baseline_samples,
            baseline_nonempty,
            baseline_accuracy,
        ),
        windowed: method_report(
            "two_layer_window_cpu",
            window_samples,
            window_nonempty,
            window_accuracy,
        ),
        prediction_disagreements,
        shots: outcomes,
        fpga: "not measured; no parallel FPGA window scheduler is implemented",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decoding::prepare_decoder;
    use crate::window::build_window_graphs;
    use crate::{Dem, convert};

    #[test]
    fn compares_real_seam_shots_on_the_same_global_graph() {
        let ir: Dem =
            serde_json::from_str(include_str!("../demo/public/fixtures/seam.ir.json")).unwrap();
        let graph = convert(ir, 100.0, false).unwrap();
        let decoder = prepare_decoder(&graph, &build_window_graphs(&graph).unwrap()).unwrap();
        let batch: ShotBatch =
            serde_json::from_str(include_str!("../examples/seam-shots.json")).unwrap();
        let report = compare_cpu_decoders(&graph, &decoder, &batch, 1, 2).unwrap();
        assert_eq!(report.graph.detectors, 3);
        assert_eq!(report.settings.shot_count, 5);
        assert_eq!(report.settings.nonempty_shot_count, 4);
        assert_eq!(report.settings.samples_per_method, 10);
        assert_eq!(report.baseline.samples_ns.len(), 10);
        assert_eq!(report.windowed.samples_ns.len(), 10);
        assert_eq!(report.baseline.shots_with_truth, 5);
        assert_eq!(report.windowed.shots_with_truth, 5);
        for method in [&report.baseline, &report.windowed] {
            assert_eq!(method.logical_accuracy.all_shots.correct, 5);
            assert_eq!(method.logical_accuracy.all_shots.total, 5);
            assert_eq!(method.logical_accuracy.all_shots.fraction, Some(1.0));
            assert_eq!(method.logical_accuracy.nonempty_shots.correct, 4);
            assert_eq!(method.logical_accuracy.nonempty_shots.total, 4);
            assert_eq!(method.logical_accuracy.nonempty_shots.fraction, Some(1.0));
        }
        assert!(
            report
                .shots
                .iter()
                .all(|shot| shot.actual_observables.is_some())
        );
        assert_eq!(report.shots[0].defects, vec![0, 2]);
        assert_eq!(report.shots[0].window_edges, vec![0, 1]);
        assert_eq!(
            report.baseline.nonempty_shots.as_ref().unwrap().min_ns,
            report
                .baseline
                .samples_ns
                .iter()
                .enumerate()
                .filter(|(index, _)| index % 5 < 4)
                .map(|(_, &value)| value)
                .min()
                .unwrap()
        );
    }

    #[test]
    fn accuracy_uses_only_truth_labeled_input_shots_once() {
        let ir: Dem =
            serde_json::from_str(include_str!("../demo/public/fixtures/seam.ir.json")).unwrap();
        let graph = convert(ir, 100.0, false).unwrap();
        let decoder = prepare_decoder(&graph, &build_window_graphs(&graph).unwrap()).unwrap();
        let batch: ShotBatch = serde_json::from_value(json!({"shots": [
            {"defects": [0, 2], "observables": []},
            {"defects": [0, 1]},
            {"defects": [], "observables": []}
        ]}))
        .unwrap();
        let report = compare_cpu_decoders(&graph, &decoder, &batch, 1, 3).unwrap();
        assert_eq!(report.settings.samples_per_method, 9);
        for method in [&report.baseline, &report.windowed] {
            assert_eq!(method.shots_with_truth, 2);
            assert_eq!(method.logical_errors, 1);
            assert_eq!(method.logical_accuracy.all_shots.correct, 1);
            assert_eq!(method.logical_accuracy.all_shots.total, 2);
            assert_eq!(method.logical_accuracy.all_shots.fraction, Some(0.5));
            assert_eq!(method.logical_accuracy.nonempty_shots.correct, 0);
            assert_eq!(method.logical_accuracy.nonempty_shots.total, 1);
            assert_eq!(method.logical_accuracy.nonempty_shots.fraction, Some(0.0));
        }
    }

    #[test]
    fn rejects_invalid_shots_or_unbounded_measurements() {
        let ir: Dem =
            serde_json::from_str(include_str!("../demo/public/fixtures/seam.ir.json")).unwrap();
        let graph = convert(ir, 100.0, false).unwrap();
        let decoder = prepare_decoder(&graph, &build_window_graphs(&graph).unwrap()).unwrap();
        let invalid: ShotBatch =
            serde_json::from_value(json!({"shots":[{"defects":[0,0]}]})).unwrap();
        assert!(
            compare_cpu_decoders(&graph, &decoder, &invalid, 0, 2)
                .unwrap_err()
                .contains("duplicate detector")
        );
        let empty: ShotBatch = serde_json::from_value(json!({"shots":[]})).unwrap();
        assert!(compare_cpu_decoders(&graph, &decoder, &empty, 0, 2).is_err());
        let valid: ShotBatch = serde_json::from_value(json!({"shots":[{"defects":[]}]})).unwrap();
        assert!(compare_cpu_decoders(&graph, &decoder, &valid, 0, 0).is_err());
        assert!(compare_cpu_decoders(&graph, &decoder, &valid, 21, 1).is_err());
        let empty_report = compare_cpu_decoders(&graph, &decoder, &valid, 0, 1).unwrap();
        assert!(empty_report.baseline.nonempty_shots.is_none());
        assert!(empty_report.windowed.nonempty_shots.is_none());
        for method in [&empty_report.baseline, &empty_report.windowed] {
            assert_eq!(method.logical_accuracy.all_shots.correct, 0);
            assert_eq!(method.logical_accuracy.all_shots.total, 0);
            assert_eq!(method.logical_accuracy.all_shots.fraction, None);
            assert_eq!(method.logical_accuracy.nonempty_shots.total, 0);
            assert_eq!(method.logical_accuracy.nonempty_shots.fraction, None);
        }
    }
}
