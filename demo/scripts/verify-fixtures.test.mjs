import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { traceShot } from '../src/model.ts';
import { globalGraphView } from '../src/globalGraph.ts';
import { shotsFromCorrectionCapture } from './benchmark-shots.mjs';

const repo = fileURLToPath(new URL('../..', import.meta.url));
const fixtures = path.join(repo, 'demo/public/fixtures');
const json = name => JSON.parse(readFileSync(path.join(fixtures, name), 'utf8'));
const xor = (initial, groups) => {
  const bits = new Set(initial);
  for (const ids of groups) for (const id of ids) bits.has(id) ? bits.delete(id) : bits.add(id);
  return [...bits].sort((a, b) => a - b);
};

for (const [name, source, count] of [
  ['tiny', 'examples/tiny.stim', 24],
  ['seam', 'examples/seam.dem', 5],
]) {
  test(`${name} captures come from the checked-in Stim input and align across all artifacts`, () => {
    const ir = json(`${name}.ir.json`);
    const graph = json(`${name}.graph.json`);
    const cut = json(`${name}.windows.json`);
    const batch = json(`${name}.corrections.json`);
    const syndromes = readFileSync(path.join(fixtures, `${name}.syndromes`), 'utf8');
    const digest = createHash('sha256').update(readFileSync(path.join(repo, source))).digest('hex');

    assert.equal(ir.provenance.input_sha256, digest);
    assert.equal(graph.num_detectors, ir.num_detectors);
    assert.equal(graph.num_observables, ir.num_observables);
    assert.equal(graph.edge_metadata.length, graph.initializer.weighted_edges.length);
    assert.equal(batch.shots.length, count);
    assert.equal(cut.windows.length, 2);
    assert.deepEqual(cut.windows.map(window => [window.layer, window.temporal_boundary]), [[0, 'open'], [1, 'closed']]);
    assert.match(syndromes, /^Syndrome Pattern v1\.0/);

    for (const window of cut.windows) {
      assert.equal(window.detector_map.length, window.initializer.vertex_num);
      assert.equal(window.edge_map.length, window.initializer.weighted_edges.length);
      for (const [localIndex, edgeId] of window.edge_map.entries()) {
        assert.ok(edgeId >= 0 && edgeId < graph.edge_metadata.length);
        const [left, right, weight] = window.initializer.weighted_edges[localIndex];
        assert.equal(weight, graph.initializer.weighted_edges[edgeId][2]);
        assert.ok(left < window.detector_map.length && right < window.detector_map.length);
      }
    }

    for (const shot of batch.shots) {
      const edges = shot.committed_edges.map(id => graph.edge_metadata[id]);
      assert.deepEqual(xor(shot.defects, edges.map(edge => edge.detectors)), shot.residual_syndrome);
      assert.deepEqual(xor([], edges.map(edge => edge.observables)), shot.predicted_observables);
      assert.deepEqual(shot.residual_syndrome, []);
      assert.equal(shot.logical_error, false);
      assert.deepEqual(shot.predicted_observables, shot.actual_observables);
      const trace = traceShot({ ir, graph, windows: cut, corrections: batch, syndromes }, shot);
      assert.deepEqual(trace.finalResidual, []);
      assert.deepEqual([...trace.topEdges, ...trace.bottomEdges].sort((a, b) => a - b), shot.committed_edges);
    }
  });
}

test('benchmark input contains only real captured defects and actual Stim/supplied observables', () => {
  for (const name of ['tiny', 'seam']) {
    const capture = json(`${name}.corrections.json`);
    const inputs = shotsFromCorrectionCapture(capture);
    assert.equal(inputs.shots.length, capture.shots.length);
    for (const [index, input] of inputs.shots.entries()) {
      assert.deepEqual(input.defects, capture.shots[index].defects);
      assert.deepEqual(input.observables, capture.shots[index].actual_observables);
      assert.equal(Object.keys(input).length, 2);
    }
    capture.shots[0].predicted_observables = [999];
    assert.deepEqual(shotsFromCorrectionCapture(capture), inputs); // predictions never become ground truth
    delete capture.shots[0].actual_observables;
    assert.throws(() => shotsFromCorrectionCapture(capture), /actual observables/);
  }
});

test('one global graph retains full edges while window bands share detector IDs', () => {
  for (const name of ['seam', 'tiny']) {
    const artifacts = {
      ir: json(`${name}.ir.json`),
      graph: json(`${name}.graph.json`),
      windows: json(`${name}.windows.json`),
      corrections: json(`${name}.corrections.json`),
      syndromes: readFileSync(path.join(fixtures, `${name}.syndromes`), 'utf8'),
    };
    const shot = name === 'seam' ? artifacts.corrections.shots[0] : artifacts.corrections.shots[9];
    const trace = traceShot(artifacts, shot);
    const before = globalGraphView(artifacts, shot, trace, 2);
    const afterTop = globalGraphView(artifacts, shot, trace, 3);
    const afterBottom = globalGraphView(artifacts, shot, trace, 4);

    assert.deepEqual(before.vertices.map(vertex => vertex.id),
      Array.from({ length: artifacts.graph.initializer.vertex_num }, (_, id) => id));
    assert.deepEqual(before.edges.map(edge => [edge.from, edge.to, edge.weight]), artifacts.graph.initializer.weighted_edges);
    assert.deepEqual(before.edges.map(edge => edge.id), artifacts.graph.edge_metadata.map((_, id) => id));
    assert.equal(before.vertices.filter(vertex => vertex.kind === 'detector').length, artifacts.graph.num_detectors);
    assert.equal(before.vertices.filter(vertex => vertex.kind === 'physical').length, 1);
    assert.ok(before.edges.every(edge => edge.committedIn === null));
    assert.deepEqual(before.syndrome, shot.defects);
    assert.deepEqual(afterTop.syndrome, trace.topResidual);
    assert.deepEqual(afterBottom.syndrome, trace.finalResidual);
    assert.deepEqual(afterTop.edges.filter(edge => edge.committedIn === 0).map(edge => edge.id), trace.topEdges);
    assert.deepEqual(afterBottom.edges.filter(edge => edge.committedIn === 1).map(edge => edge.id), trace.bottomEdges);
    assert.deepEqual(afterBottom.edges.filter(edge => edge.committedIn === 0).map(edge => edge.id), trace.topEdges);
    assert.ok(before.windows[0].buffer.includes(1) && before.windows[1].commit.includes(1));

    if (name === 'seam') {
      assert.deepEqual(before.edges.filter(edge => edge.topLocalTemporal).map(edge => edge.id), [1]);
      assert.deepEqual([before.edges[1].from, before.edges[1].to], [1, 2]); // Global e1 is never cut here.
      assert.deepEqual(afterTop.syndrome, [1, 2]);
    } else {
      assert.deepEqual(before.edges.filter(edge => edge.topLocalTemporal), []); // Both edges end at the same physical V.
      assert.equal(before.edges[0].to, before.edges[1].to);
    }
  }
});

test('supplied seam shot actually passes a residual from the open top to the closed bottom', () => {
  const graph = json('seam.graph.json');
  const cut = json('seam.windows.json');
  const shot = json('seam.corrections.json').shots[0];
  assert.deepEqual(shot.defects, [0, 2]);
  assert.deepEqual(shot.committed_edges, [0, 1]);
  assert.ok(cut.windows[0].edge_map.includes(0) && cut.windows[1].edge_map.includes(1));
  const afterTop = xor(shot.defects, [graph.edge_metadata[0].detectors]);
  assert.deepEqual(afterTop, [1, 2]);
  assert.deepEqual(xor(afterTop, [graph.edge_metadata[1].detectors]), []);
  const trace = traceShot({ graph, windows: cut }, shot);
  assert.deepEqual(trace.topEdges, [0]);
  assert.deepEqual(trace.bottomEdges, [1]);
  assert.deepEqual(trace.topResidual, [1, 2]);
});
