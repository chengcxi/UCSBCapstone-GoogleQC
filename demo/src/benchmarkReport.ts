import { parityToggle, type Graph } from './model.ts';

export interface LatencyStats {
  median_ns: number;
  p95_ns: number;
  mean_ns: number;
  min_ns: number;
  max_ns: number;
}

export interface AccuracyStats {
  correct: number;
  total: number;
  fraction: number | null;
}

export interface AccuracyBreakdown {
  all_shots: AccuracyStats;
  nonempty_shots: AccuracyStats;
}

export interface BenchmarkMethod {
  id: 'micro_blossom_global_cpu' | 'two_layer_window_cpu';
  execution: 'sequential_cpu';
  all_shots: LatencyStats;
  nonempty_shots: LatencyStats | null;
  samples_ns: number[];
  shots_with_truth: number;
  logical_errors: number;
  /** Additive schema-v1 field. Earlier real CLI captures omit it; compute from their shot rows. */
  logical_accuracy?: AccuracyBreakdown;
}

export interface BenchmarkShot {
  index: number;
  defects: number[];
  actual_observables: number[] | null;
  baseline_edges: number[];
  window_edges: number[];
  baseline_observables: number[];
  window_observables: number[];
  same_prediction: boolean;
}

export interface BenchmarkReport {
  schema_version: 1;
  kind: 'cpu_decoder_comparison';
  provenance: { input_sha256?: string; stim_version?: string; input_kind?: string } | null;
  graph: { detectors: number; edges: number; windows: number };
  host: { os: string; arch: string; logical_cpus: number | null; profile: 'debug' | 'release' };
  settings: {
    warmup_passes: number; measured_passes: number; shot_count: number;
    nonempty_shot_count: number; samples_per_method: number;
    timing_scope: string; order: string;
  };
  baseline: BenchmarkMethod;
  windowed: BenchmarkMethod;
  prediction_disagreements: number;
  shots: BenchmarkShot[];
  fpga: string;
}

function accuracyFromRows(rows: BenchmarkShot[], method: 'baseline' | 'windowed', nonemptyOnly: boolean): AccuracyStats {
  let correct = 0;
  let total = 0;
  for (const shot of rows) {
    if (nonemptyOnly && shot.defects.length === 0) continue;
    if (shot.actual_observables === null) continue;
    total++;
    const predicted = method === 'baseline' ? shot.baseline_observables : shot.window_observables;
    if (JSON.stringify(predicted) === JSON.stringify(shot.actual_observables)) correct++;
  }
  return { correct, total, fraction: total ? correct / total : null };
}

function accuracyBreakdown(rows: BenchmarkShot[], method: 'baseline' | 'windowed'): AccuracyBreakdown {
  return {
    all_shots: accuracyFromRows(rows, method, false),
    nonempty_shots: accuracyFromRows(rows, method, true),
  };
}

/** Accuracy uses one vote per truth-labeled shot, never the timed repeat count.
 * Older saved reports omit the optional aggregate but still contain the real
 * per-shot predictions and ground truth needed to derive it exactly. */
export function methodAccuracy(report: BenchmarkReport, method: 'baseline' | 'windowed', nonemptyOnly: boolean): AccuracyStats {
  const population = nonemptyOnly ? 'nonempty_shots' : 'all_shots';
  return report[method].logical_accuracy?.[population]
    ?? accuracyFromRows(report.shots, method, nonemptyOnly);
}

export function accuracyLabel(stats: AccuracyStats): string {
  return stats.fraction === null ? 'N/A (no truth)' : `${(stats.fraction * 100).toFixed(1)}% (${stats.correct}/${stats.total})`;
}

function record(value: unknown, label: string): Record<string, unknown> {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) throw new Error(`Invalid ${label}`);
  return value as Record<string, unknown>;
}
function integer(value: unknown, label: string, min = 0): number {
  if (typeof value !== 'number' || !Number.isSafeInteger(value) || value < min) throw new Error(`Invalid ${label}`);
  return value;
}
function string(value: unknown, label: string): string {
  if (typeof value !== 'string') throw new Error(`Invalid ${label}`);
  return value;
}
function ids(value: unknown, label: string): number[] {
  if (!Array.isArray(value) || value.some(id => !Number.isSafeInteger(id) || id < 0)
      || new Set(value).size !== value.length) throw new Error(`Invalid ${label}`);
  return value as number[];
}
function numbers(value: unknown, label: string, expected: number): number[] {
  if (!Array.isArray(value) || value.length !== expected || value.some(item => !Number.isSafeInteger(item) || item < 0)) {
    throw new Error(`Invalid ${label}: expected ${expected} nonnegative nanosecond samples`);
  }
  return value as number[];
}
function computedStats(samples: number[]): LatencyStats {
  const sorted = [...samples].sort((a, b) => a - b);
  const n = sorted.length;
  return {
    median_ns: (sorted[Math.floor((n - 1) / 2)] + sorted[Math.floor(n / 2)]) / 2,
    p95_ns: sorted[Math.ceil(n * .95) - 1],
    mean_ns: sorted.reduce((sum, value) => sum + value, 0) / n,
    min_ns: sorted[0], max_ns: sorted[n - 1],
  };
}
function checkStats(value: unknown, samples: number[], label: string): LatencyStats {
  const item = record(value, label);
  const expected = computedStats(samples);
  for (const key of Object.keys(expected) as (keyof LatencyStats)[]) {
    const actual = item[key];
    if (typeof actual !== 'number' || !Number.isFinite(actual) || actual < 0
        || Math.abs(actual - expected[key]) > Math.max(.001, Math.abs(expected[key]) * 1e-9)) {
      throw new Error(`${label}.${key} does not match the measured samples`);
    }
  }
  return item as unknown as LatencyStats;
}

/** Imported files are user-provided measurements, not trusted fixtures. Reject
 * inconsistent summaries, incorrect shot counts or an invented FPGA number. */
export function parseBenchmarkReport(value: unknown, sourceGraph?: Graph): BenchmarkReport {
  const item = record(value, 'benchmark report');
  if (item.schema_version !== 1 || item.kind !== 'cpu_decoder_comparison') {
    throw new Error('Not a schema-v1 Micro Blossom CPU comparison report');
  }
  const graph = record(item.graph, 'graph size');
  integer(graph.detectors, 'detector count', 1); integer(graph.edges, 'edge count', 1);
  integer(graph.windows, 'window count', 1);
  if (sourceGraph && (graph.detectors !== sourceGraph.num_detectors
      || graph.edges !== sourceGraph.edge_metadata.length
      || graph.windows !== sourceGraph.window_annotations.windows.length)) {
    throw new Error('The report graph does not match the selected fixture');
  }
  const settings = record(item.settings, 'settings');
  const shotCount = integer(settings.shot_count, 'shot count', 1);
  const nonempty = integer(settings.nonempty_shot_count, 'nonempty count');
  const measured = integer(settings.measured_passes, 'measured passes', 1);
  integer(settings.warmup_passes, 'warmup passes');
  if (shotCount > 1_000 || measured > 1_000 || nonempty > shotCount
      || settings.samples_per_method !== shotCount * measured || shotCount * measured > 50_000) {
    throw new Error('Inconsistent or oversized benchmark sample count');
  }
  string(settings.timing_scope, 'timing scope'); string(settings.order, 'execution order');
  const host = record(item.host, 'host');
  string(host.os, 'host OS'); string(host.arch, 'host architecture');
  if (host.profile !== 'debug' && host.profile !== 'release') throw new Error('Unknown build profile');
  if (host.logical_cpus !== null) integer(host.logical_cpus, 'logical CPU count', 1);
  if (item.provenance !== null) record(item.provenance, 'provenance');
  const rows = item.shots;
  if (!Array.isArray(rows) || rows.length !== shotCount) throw new Error('Shot comparison count does not match settings');
  let countNonempty = 0;
  let countTruth = 0;
  let baselineErrors = 0;
  let windowErrors = 0;
  let disagreements = 0;
  rows.forEach((raw: unknown, index: number) => {
    const shot = record(raw, `shot ${index}`);
    if (shot.index !== index) throw new Error(`Shot ${index} has a mismatched index`);
    const defects = ids(shot.defects, `shot ${index} defects`);
    if (defects.some(id => id >= (graph.detectors as number))) throw new Error(`Shot ${index} has an out-of-range detector`);
    if (defects.length) countNonempty++;
    const baselineEdges = ids(shot.baseline_edges, `shot ${index} global edges`);
    const windowEdges = ids(shot.window_edges, `shot ${index} window edges`);
    const original = ids(shot.baseline_observables, `shot ${index} baseline observables`);
    const windowed = ids(shot.window_observables, `shot ${index} window observables`);
    if (sourceGraph) {
      for (const [edges, predicted] of [[baselineEdges, original], [windowEdges, windowed]]) {
        if (edges.some(id => id >= sourceGraph.edge_metadata.length)) {
          throw new Error(`Shot ${index} has an out-of-range correction edge`);
        }
        if (parityToggle(defects, edges.map(id => sourceGraph.edge_metadata[id].detectors)).length
            || JSON.stringify(parityToggle([], edges.map(id => sourceGraph.edge_metadata[id].observables))) !== JSON.stringify(predicted)) {
          throw new Error(`Shot ${index} correction fails the global residual or observable parity check`);
        }
      }
    }
    const same = JSON.stringify(original) === JSON.stringify(windowed);
    if (shot.same_prediction !== same) throw new Error(`Shot ${index} has an inconsistent prediction comparison`);
    if (!same) disagreements++;
    if (shot.actual_observables !== null) {
      const actual = ids(shot.actual_observables, `shot ${index} actual observables`);
      countTruth++;
      if (JSON.stringify(original) !== JSON.stringify(actual)) baselineErrors++;
      if (JSON.stringify(windowed) !== JSON.stringify(actual)) windowErrors++;
    }
  });
  if (countNonempty !== nonempty || item.prediction_disagreements !== disagreements) {
    throw new Error('Shot counts or prediction disagreements do not match the report');
  }
  const expectedSamples = shotCount * measured;
  const checkMethod = (raw: unknown, id: BenchmarkMethod['id'], errors: number) => {
    const method = record(raw, id);
    if (method.id !== id || method.execution !== 'sequential_cpu'
        || method.shots_with_truth !== countTruth || method.logical_errors !== errors) {
      throw new Error(`Invalid ${id} identity or logical error count`);
    }
    if (method.logical_accuracy !== undefined) {
      const accuracy = record(method.logical_accuracy, `${id} logical accuracy`);
      const expected = accuracyBreakdown(rows as BenchmarkShot[], id === 'micro_blossom_global_cpu' ? 'baseline' : 'windowed');
      for (const scope of ['all_shots', 'nonempty_shots'] as const) {
        const stats = record(accuracy[scope], `${id} ${scope} logical accuracy`);
        const reference = expected[scope];
        if (integer(stats.correct, `${id} ${scope} correct`) !== reference.correct
            || integer(stats.total, `${id} ${scope} total`) !== reference.total
            || (reference.fraction === null ? stats.fraction !== null
              : typeof stats.fraction !== 'number' || !Number.isFinite(stats.fraction)
                || Math.abs(stats.fraction - reference.fraction) > 1e-12)) {
          throw new Error(`${id} ${scope} logical accuracy does not match the reported shots`);
        }
      }
    }
    const samples = numbers(method.samples_ns, `${id} samples`, expectedSamples);
    checkStats(method.all_shots, samples, `${id}.all_shots`);
    const subset = samples.filter((_, index) => (rows[index % shotCount] as BenchmarkShot).defects.length > 0);
    if (nonempty) checkStats(method.nonempty_shots, subset, `${id}.nonempty_shots`);
    else if (method.nonempty_shots !== null) throw new Error(`${id} has no nonempty shots`);
  };
  checkMethod(item.baseline, 'micro_blossom_global_cpu', baselineErrors);
  checkMethod(item.windowed, 'two_layer_window_cpu', windowErrors);
  const fpga = string(item.fpga, 'FPGA status');
  if (!fpga.toLowerCase().includes('not measured')) throw new Error('FPGA results cannot be claimed by this CPU report');
  return item as unknown as BenchmarkReport;
}

export function latencyLabel(ns: number): string {
  if (ns >= 1_000_000) return `${(ns / 1_000_000).toFixed(3)} ms`;
  if (ns >= 1_000) return `${(ns / 1_000).toFixed(3)} µs`;
  return `${ns.toFixed(0)} ns`;
}
