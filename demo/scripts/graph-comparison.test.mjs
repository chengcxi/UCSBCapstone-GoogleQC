import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { comparisonAccuracy, parseGraphComparison, windowCounts } from '../src/graphComparison.ts';
import { parseGenerationConfig } from '../src/generationConfig.ts';
import { parityToggle } from '../src/model.ts';

const capture = JSON.parse(readFileSync(new URL('../public/fixtures/d5_1000.comparison.json', import.meta.url)));
const { graph } = capture;

test('graph imports verify real captures and reject altered topology, predictions and timing claims', () => {
  assert.equal(parseGraphComparison(capture), capture);
  for (const mutate of [
    x => { x.graph.detector_coordinates[0][2] = null; },
    x => { x.windows[0].initializer.weighted_edges[0][2] += 2; },
    x => { x.shots[0].reference_prediction = [99]; },
    x => { const row = x.shots.find(s => s.error); row.window_prediction = []; },
    x => { x.shots[0].reference_edges = []; },
    x => { x.timing = { median_ns: 1 }; },
    x => { x.graph.window_annotations.windows[0].commit.push(999999); },
  ]) {
    const changed = structuredClone(capture); mutate(changed);
    assert.throws(() => parseGraphComparison(changed), /Invalid graph capture/);
  }
});

test('generation accepts bounded numerical settings and rejects paths, commands and oversized workloads', () => {
  const config = { distance: 3, rounds: 6, shots: 32, noise: 0.003, seed: 7,
    basis: 'z', commit: 3, buffer: 3, measure: false, warmup: 0, repeats: 1 };
  assert.deepEqual(parseGenerationConfig(config), config);
  for (const changes of [ { distance: 4 }, { shots: 257 }, { seed: -1 }, { noise: 0 },
    { rounds: 90, distance: 7 }, { distance: 7, commit: 7, buffer: 7 },
    { rounds: 2, commit: 15 }, { binary: '/tmp/command' }, { measure: 'yes' }, { shots: 256, repeats: 20 },
  ]) assert.throws(() => parseGenerationConfig({...config, ...changes}));
});

test('d5_1000 preserves rejected shots and identifies the actual reference without inventing timings', () => {
  assert.equal(capture.kind, 'recorded_window_graph_comparison');
  assert.equal(capture.timing, null);
  assert.equal(capture.summary.timings_seconds, undefined);
  assert.match(capture.reference_method, /Fusion Blossom SolverSerial/);
  assert.equal(graph.num_detectors, 1416);
  assert.equal(graph.initializer.weighted_edges.length, 6550);
  assert.equal(capture.windows.length, 12);
  assert.deepEqual(comparisonAccuracy(capture.shots), {
    total: 1000, referenceCorrect: 959, accepted: 208, windowCorrect: 200, rejected: 792,
  });
});

test('all local graph edges retain global weights and have exactly the exported mapped endpoints', () => {
  for (const cut of capture.windows) {
    const virtuals = new Set(cut.initializer.virtual_vertices);
    assert.equal(cut.edge_map.length, cut.initializer.weighted_edges.length);
    assert.equal(cut.detector_map.length, cut.initializer.vertex_num);
    for (const [local, global] of cut.edge_map.entries()) {
      const [l, r, weight] = cut.initializer.weighted_edges[local];
      assert.equal(weight, graph.initializer.weighted_edges[global][2]);
      const real = [l, r].filter(v => !virtuals.has(v)).map(v => cut.detector_map[v]);
      const support = graph.edge_metadata[global].detectors;
      assert.ok(real.every(d => support.includes(d)));
      if (!virtuals.has(l) && !virtuals.has(r)) assert.deepEqual(real.sort((a,b) => a-b), [...support].sort((a,b) => a-b));
    }
    if (cut.layer === 1) assert.equal(windowCounts(graph, cut).rewired, 0);
  }
  const top = capture.windows.find(w => w.window_id === 'top-1');
  assert.equal(windowCounts(graph, top).detectors, 360);
  assert.equal(windowCounts(graph, top).edges, 1750);
  assert.ok(windowCounts(graph, top).rewired > 0);
});

test('reference and accepted window corrections clear the syndrome and predict their recorded observables', () => {
  for (const shot of capture.shots) {
    for (const [edges, predicted] of [[shot.reference_edges, shot.reference_prediction], [shot.window_edges, shot.window_prediction]]) {
      if (edges === null) continue;
      assert.deepEqual(parityToggle(shot.defects, edges.map(e => graph.edge_metadata[e].detectors)), []);
      assert.deepEqual(parityToggle([], edges.map(e => graph.edge_metadata[e].observables)), predicted);
    }
    if (shot.error) {
      assert.equal(shot.window_edges, null);
      assert.equal(shot.window_prediction, null);
      assert.ok(shot.unmatched.length > 0);
    }
  }
});

test('the rejected residual capture supports the displayed top-commit seam diagnosis', () => {
  const times = graph.detector_coordinates.map(c => c[2]);
  const starts = new Set(graph.window_annotations.windows.filter(w => w.layer === 0)
    .map(w => Math.min(...w.commit.map(d => times[d]))));
  for (const shot of capture.shots) for (const detector of shot.unmatched) assert.ok(starts.has(times[detector]));
});
