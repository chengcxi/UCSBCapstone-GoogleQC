import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { accuracyLabel, latencyLabel, methodAccuracy, parseBenchmarkReport } from '../src/benchmarkReport.ts';
import { shotsFromCorrectionCapture } from '../src/benchmarkShots.ts';

// Synthetic values exist in this validator test only; the UI never ships a
// fabricated report or benchmark timings.
function report() {
  return {
    schema_version: 1, kind: 'cpu_decoder_comparison',
    provenance: { input_sha256: 'test-only' },
    graph: { detectors: 1, edges: 1, windows: 2 },
    host: { os: 'test', arch: 'test', logical_cpus: 1, profile: 'release' },
    settings: {
      warmup_passes: 1, measured_passes: 2, shot_count: 2, nonempty_shot_count: 1,
      samples_per_method: 4, timing_scope: 'test only', order: 'alternated',
    },
    baseline: {
      id: 'micro_blossom_global_cpu', execution: 'sequential_cpu', samples_ns: [100, 20, 110, 22],
      all_shots: { median_ns: 61, p95_ns: 110, mean_ns: 63, min_ns: 20, max_ns: 110 },
      nonempty_shots: { median_ns: 105, p95_ns: 110, mean_ns: 105, min_ns: 100, max_ns: 110 },
      shots_with_truth: 2, logical_errors: 0,
    },
    windowed: {
      id: 'two_layer_window_cpu', execution: 'sequential_cpu', samples_ns: [140, 30, 150, 32],
      all_shots: { median_ns: 86, p95_ns: 150, mean_ns: 88, min_ns: 30, max_ns: 150 },
      nonempty_shots: { median_ns: 145, p95_ns: 150, mean_ns: 145, min_ns: 140, max_ns: 150 },
      shots_with_truth: 2, logical_errors: 0,
    },
    prediction_disagreements: 0,
    shots: [
      { index: 0, defects: [0], actual_observables: [0], baseline_edges: [0], window_edges: [0],
        baseline_observables: [0], window_observables: [0], same_prediction: true },
      { index: 1, defects: [], actual_observables: [], baseline_edges: [], window_edges: [],
        baseline_observables: [], window_observables: [], same_prediction: true },
    ],
    fpga: 'not measured; no parallel FPGA window scheduler is implemented',
  };
}

test('committed CI report is a release measurement of the same captured seam inputs', () => {
  const graph = JSON.parse(readFileSync(new URL('../public/fixtures/seam.graph.json', import.meta.url)));
  const sample = parseBenchmarkReport(JSON.parse(readFileSync(new URL('../public/fixtures/seam.benchmark.json', import.meta.url))), graph);
  const ir = JSON.parse(readFileSync(new URL('../public/fixtures/seam.ir.json', import.meta.url)));
  const capture = JSON.parse(readFileSync(new URL('../public/fixtures/seam.corrections.json', import.meta.url)));
  const inputs = shotsFromCorrectionCapture(capture).shots;
  assert.equal(sample.host.profile, 'release');
  assert.equal(sample.provenance.input_sha256, ir.provenance.input_sha256);
  assert.equal(sample.settings.shot_count, inputs.length);
  assert.equal(sample.settings.warmup_passes, 2);
  assert.equal(sample.settings.measured_passes, 20);
  for (const method of ['baseline', 'windowed']) {
    assert.deepEqual(sample[method].logical_accuracy?.all_shots, { correct: 5, total: 5, fraction: 1 });
    assert.deepEqual(sample[method].logical_accuracy?.nonempty_shots, { correct: 4, total: 4, fraction: 1 });
    assert.deepEqual(methodAccuracy(sample, method, false), { correct: 5, total: 5, fraction: 1 });
    assert.deepEqual(methodAccuracy(sample, method, true), { correct: 4, total: 4, fraction: 1 });
  }
  for (const [index, shot] of sample.shots.entries()) {
    assert.deepEqual(shot.defects, inputs[index].defects);
    assert.deepEqual(shot.actual_observables, inputs[index].observables);
  }
  const forged = structuredClone(sample);
  forged.shots[0].window_edges = [];
  assert.throws(() => parseBenchmarkReport(forged, graph), /global residual/);
});

test('accepts complete CPU report and preserves sample-based statistics', () => {
  assert.equal(parseBenchmarkReport(report()).windowed.nonempty_shots.median_ns, 145);
  assert.equal(latencyLabel(1450), '1.450 µs');
});

test('logical accuracy counts labeled shots once, respects nonempty view, and is N/A without truth', () => {
  const legacy = parseBenchmarkReport(report()); // old schema-v1 report has no aggregate accuracy field
  assert.deepEqual(methodAccuracy(legacy, 'baseline', false), { correct: 2, total: 2, fraction: 1 });
  assert.deepEqual(methodAccuracy(legacy, 'windowed', true), { correct: 1, total: 1, fraction: 1 });
  assert.equal(accuracyLabel(methodAccuracy(legacy, 'baseline', false)), '100.0% (2/2)');

  const differentPopulations = report();
  differentPopulations.shots[0].actual_observables = [];
  differentPopulations.baseline.logical_errors = 1;
  differentPopulations.windowed.logical_errors = 1;
  const split = parseBenchmarkReport(differentPopulations);
  assert.deepEqual(methodAccuracy(split, 'baseline', false), { correct: 1, total: 2, fraction: 0.5 });
  assert.deepEqual(methodAccuracy(split, 'baseline', true), { correct: 0, total: 1, fraction: 0 });

  const partial = report();
  partial.shots[0].actual_observables = [];
  partial.shots[1].actual_observables = null;
  for (const method of [partial.baseline, partial.windowed]) {
    method.shots_with_truth = 1;
    method.logical_errors = 1;
  }
  const checked = parseBenchmarkReport(partial);
  assert.deepEqual(methodAccuracy(checked, 'baseline', false), { correct: 0, total: 1, fraction: 0 });
  assert.deepEqual(methodAccuracy(checked, 'windowed', true), { correct: 0, total: 1, fraction: 0 });
  assert.equal(accuracyLabel(methodAccuracy(checked, 'windowed', true)), '0.0% (0/1)');

  partial.shots[0].actual_observables = null;
  for (const method of [partial.baseline, partial.windowed]) {
    method.shots_with_truth = 0;
    method.logical_errors = 0;
  }
  const unknown = parseBenchmarkReport(partial);
  assert.deepEqual(methodAccuracy(unknown, 'baseline', false), { correct: 0, total: 0, fraction: null });
  assert.equal(accuracyLabel(methodAccuracy(unknown, 'baseline', true)), 'N/A (no truth)');
});

test('separately scores the global and window CPU predictions', () => {
  const diverged = report();
  diverged.shots[0].window_observables = [];
  diverged.shots[0].same_prediction = false;
  diverged.prediction_disagreements = 1;
  diverged.windowed.logical_errors = 1;
  const checked = parseBenchmarkReport(diverged);
  assert.deepEqual(methodAccuracy(checked, 'baseline', true), { correct: 1, total: 1, fraction: 1 });
  assert.deepEqual(methodAccuracy(checked, 'windowed', true), { correct: 0, total: 1, fraction: 0 });
});

test('validates explicit CPU accuracy fields instead of trusting a claimed percentage', () => {
  const candidate = report();
  for (const method of [candidate.baseline, candidate.windowed]) {
    method.logical_accuracy = {
      all_shots: { correct: 2, total: 2, fraction: 1 },
      nonempty_shots: { correct: 1, total: 1, fraction: 1 },
    };
  }
  assert.deepEqual(methodAccuracy(parseBenchmarkReport(candidate), 'baseline', true),
    { correct: 1, total: 1, fraction: 1 });
  candidate.baseline.logical_accuracy.nonempty_shots.fraction = 0.75;
  assert.throws(() => parseBenchmarkReport(candidate), /logical accuracy/);
  candidate.baseline.logical_accuracy.nonempty_shots.fraction = 1;
  candidate.windowed.logical_accuracy.all_shots.total = 4;
  assert.throws(() => parseBenchmarkReport(candidate), /logical accuracy/);
});

test('rejects forged summaries, missing data, or FPGA performance claims', () => {
  const forged = report();
  forged.baseline.all_shots.median_ns = 1;
  assert.throws(() => parseBenchmarkReport(forged), /measured samples/);
  const short = report();
  short.windowed.samples_ns.pop();
  assert.throws(() => parseBenchmarkReport(short), /expected 4/);
  const fpga = report();
  fpga.fpga = '100 ns';
  assert.throws(() => parseBenchmarkReport(fpga), /FPGA results/);
  const mismatched = report();
  mismatched.shots[0].same_prediction = false;
  assert.throws(() => parseBenchmarkReport(mismatched), /prediction comparison/);
});
