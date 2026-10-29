import { parityToggle, type Graph, type WindowGraph } from './model.ts';
import { parseBenchmarkReport, type BenchmarkReport } from './benchmarkReport.ts';
import type { GenerationConfig } from './generationConfig';

export interface ComparisonShot {
  index: number;
  defects: number[];
  actual: number[];
  reference_edges: number[];
  reference_prediction: number[];
  window_edges: number[] | null;
  window_prediction: number[] | null;
  error: string | null;
  unmatched: number[];
}

export interface GraphComparison {
  schema_version: 1;
  kind: 'recorded_window_graph_comparison';
  reference_method: string;
  window_method: string;
  timing: null;
  summary: { distance: number; rounds: number; seed: number; noise_probability_each_channel: number;
    git_revision: string; correlated_components_approximated: boolean; basis?: 'x' | 'z'; commit_slices?: number;
    buffer_slices_each_side?: number };
  graph: Graph;
  windows: WindowGraph[];
  shots: ComparisonShot[];
  cpu_benchmark?: BenchmarkReport | null;
  cpu_benchmark_error?: string | null;
  source_circuit?: string;
  generation_settings?: GenerationConfig;
}

/** Validate topology and correction parity before admitting an imported dataset.
 * Imported execution/provenance claims remain user-provided, not attested. */
export function parseGraphComparison(value: unknown): GraphComparison {
  function fail(message: string): never { throw new Error(`Invalid graph capture: ${message}`); }
  const record = (v: unknown): Record<string, unknown> => v !== null && typeof v === 'object' && !Array.isArray(v) ? v as Record<string, unknown> : fail('expected an object');
  const integer = (v: unknown, max = 1000000): number => typeof v === 'number' && Number.isSafeInteger(v) && v >= 0 && v <= max ? v : fail('invalid integer');
  const array = (v: unknown): unknown[] => Array.isArray(v) ? v : fail('expected an array');
  function ids(v: unknown, limit: number): number[] {
    const items = array(v).map(x => integer(x, limit - 1));
    if (new Set(items).size !== items.length) fail('duplicate IDs');
    return items;
  }
  const root = record(value);
  if (root.schema_version !== 1 || root.kind !== 'recorded_window_graph_comparison' || root.timing !== null) fail('unsupported schema or timing claim');
  if (root.reference_method !== 'Full-graph Fusion Blossom SolverSerial'
      || root.window_method !== 'Micro Blossom ParallelWindowDecoder (sequential CPU)') fail('unsupported decoder methods');
  const s = record(root.summary);
  if (integer(s.distance, 99) < 3 || integer(s.rounds, 10000) < 1) fail('invalid circuit size');
  integer(s.seed, 4294967295);
  if (typeof s.noise_probability_each_channel !== 'number' || !Number.isFinite(s.noise_probability_each_channel)
      || s.noise_probability_each_channel <= 0 || s.noise_probability_each_channel >= 0.5
      || typeof s.git_revision !== 'string' || typeof s.correlated_components_approximated !== 'boolean') fail('invalid circuit metadata');
  if (s.basis !== undefined && s.basis !== 'x' && s.basis !== 'z') fail('memory basis');
  const graph = record(root.graph);
  const n = integer(graph.num_detectors, 10000), observables = integer(graph.num_observables, 100);
  if (!n || !observables) fail('empty graph');
  const coords = array(graph.detector_coordinates);
  if (coords.length !== n || coords.some(c => !Array.isArray(c) || c.length < 3 || c.some(x => typeof x !== 'number' || !Number.isFinite(x)))) fail('missing coordinates');
  function initializer(v: unknown) {
    const init = record(v), vertices = integer(init.vertex_num, 20000);
    const virtuals = ids(init.virtual_vertices, vertices);
    const edges = array(init.weighted_edges).map(row => {
      const edge = array(row);
      if (edge.length !== 3) fail('invalid edge');
      return [integer(edge[0], vertices - 1), integer(edge[1], vertices - 1), integer(edge[2], 2147483647)];
    });
    if (!edges.length || edges.length > 100000) fail('invalid edge count');
    return { vertices, virtuals: new Set(virtuals), edges };
  }
  const global = initializer(graph.initializer);
  if (global.vertices < n || Array.from({length: global.vertices}, (_, i) => i).some(i => (i >= n) !== global.virtuals.has(i))) fail('global boundary IDs');
  const metadata = array(graph.edge_metadata).map(v => record(v));
  if (metadata.length !== global.edges.length) fail('edge metadata length');
  metadata.forEach((edge, i) => {
    const support = ids(edge.detectors, n); ids(edge.observables, observables);
    const real = global.edges[i].slice(0, 2).filter(v => !global.virtuals.has(v));
    if (support.length < 1 || support.length > 2 || JSON.stringify([...support].sort((a,b)=>a-b)) !== JSON.stringify(real.sort((a,b)=>a-b))) fail('global edge support mismatch');
  });
  const plans = array(record(graph.window_annotations).windows).map(v => record(v));
  const ownership = new Set<number>();
  const planNames = new Set<string>();
  for (const p of plans) {
    if (typeof p.id !== 'string' || planNames.has(p.id) || (p.layer !== 0 && p.layer !== 1)) fail('invalid window plan');
    planNames.add(p.id);
    const commit = ids(p.commit, n), buffer = ids(p.buffer, n);
    if (!commit.length || (p.layer === 1 && buffer.length)) fail('invalid commit or buffer');
    for (const d of commit) { if (ownership.has(d) || buffer.includes(d)) fail('overlapping commit'); ownership.add(d); }
  }
  if (ownership.size !== n) fail('uncovered detectors');
  const cuts = array(root.windows);
  if (!cuts.length || cuts.length !== plans.length) fail('window count');
  const cutNames = new Set();
  for (const raw of cuts) {
    const cut = record(raw), plan = plans.find(p => p.id === cut.window_id);
    if (!plan || cutNames.has(cut.window_id) || cut.layer !== plan.layer || cut.temporal_boundary !== (plan.layer === 0 ? 'open' : 'closed')) fail('window identity');
    cutNames.add(cut.window_id);
    const local = initializer(cut.initializer), map = array(cut.detector_map), edgeMap = ids(cut.edge_map, global.edges.length);
    if (map.length !== local.vertices || edgeMap.length !== local.edges.length) fail('local maps');
    const members = new Set([...array(plan!.commit), ...array(plan!.buffer)]);
    const expected = metadata.flatMap((edge, i) => {
      const support = edge.detectors as number[];
      const inside = support.filter(d => members.has(d)).length;
      return inside && (inside === support.length || plan!.layer === 0) ? [i] : [];
    });
    if (expected.length !== edgeMap.length || expected.some(e => !edgeMap.includes(e))) fail('missing or extra cut edges');
    const mapped = map.filter(d => d !== null).map(d => integer(d, n - 1));
    if (new Set(mapped).size !== mapped.length || mapped.length !== members.size || mapped.some(d => !members.has(d))) fail('detector membership');
    map.forEach((d, i) => { if ((d === null) !== local.virtuals.has(i)) fail('virtual detector map'); });
    local.edges.forEach(([l,r,w], i) => {
      const globalId = edgeMap[i], support = metadata[globalId].detectors as number[];
      if (w !== global.edges[globalId][2]) fail('changed local weight');
      const real = [map[l], map[r]].filter(d => d !== null) as number[];
      if (!real.length || new Set(real).size !== real.length || real.some(d => !support.includes(d))) fail('local endpoints');
      if (real.length !== support.length && (plan!.layer !== 0 || support.length !== 2 || support.every(d => members.has(d)))) fail('invalid temporal rewiring');
    });
  }
  const rows = array(root.shots);
  if (!rows.length || rows.length > 10000) fail('shot count');
  rows.forEach((raw, i) => {
    const shot = record(raw);
    if (shot.index !== i) fail('shot indexing');
    const defects = ids(shot.defects, n); ids(shot.actual, observables); ids(shot.unmatched, n);
    const check = (edges: unknown, prediction: unknown) => {
      const edgeIds = ids(edges, global.edges.length), predicted = ids(prediction, observables);
      if (parityToggle(defects, edgeIds.map(e => metadata[e].detectors as number[])).length
          || JSON.stringify(parityToggle([], edgeIds.map(e => metadata[e].observables as number[]))) !== JSON.stringify([...predicted].sort((a,b)=>a-b))) fail('correction parity');
    };
    check(shot.reference_edges, shot.reference_prediction);
    if (shot.error !== null) {
      if (typeof shot.error !== 'string' || !shot.error || shot.window_edges !== null || shot.window_prediction !== null) fail('rejected shot representation');
    } else {
      if (array(shot.unmatched).length) fail('accepted shot residual');
      check(shot.window_edges, shot.window_prediction);
    }
  });
  if (root.source_circuit !== undefined && (typeof root.source_circuit !== 'string' || root.source_circuit.length > 2000000)) fail('circuit source');
  if (root.cpu_benchmark_error !== undefined && root.cpu_benchmark_error !== null && typeof root.cpu_benchmark_error !== 'string') fail('timing error');
  if (root.cpu_benchmark) {
    const timing = parseBenchmarkReport(root.cpu_benchmark, root.graph as Graph);
    if (timing.shots.length !== rows.length || timing.provenance?.input_sha256 !== record(graph.provenance).input_sha256) fail('timing source');
    timing.shots.forEach((shot, i) => { const input = record(rows[i]);
      if (JSON.stringify(shot.defects) !== JSON.stringify(input.defects)
          || JSON.stringify(shot.actual_observables) !== JSON.stringify(input.actual)
          || input.error !== null) fail('timing shot inputs');
    });
  }
  return value as GraphComparison;
}

export function windowCounts(graph: Graph, cut: WindowGraph) {
  const members = new Set(cut.detector_map.filter((d): d is number => d !== null));
  let physical = 0;
  let rewired = 0;
  for (const edge of cut.edge_map) {
    const support = graph.edge_metadata[edge].detectors;
    if (support.length === 1) physical++;
    else if (support.some(d => !members.has(d))) rewired++;
  }
  return { detectors: members.size, edges: cut.edge_map.length, physical, rewired,
    omitted: graph.initializer.weighted_edges.length - cut.edge_map.length };
}

export function comparisonAccuracy(shots: ComparisonShot[]) {
  const same = (a: number[], b: number[]) => JSON.stringify([...a].sort((x,y)=>x-y)) === JSON.stringify([...b].sort((x,y)=>x-y));
  return {
    total: shots.length,
    referenceCorrect: shots.filter(s => same(s.actual, s.reference_prediction)).length,
    accepted: shots.filter(s => s.window_prediction !== null).length,
    windowCorrect: shots.filter(s => s.window_prediction !== null && same(s.actual, s.window_prediction)).length,
    rejected: shots.filter(s => s.error !== null).length,
  };
}
