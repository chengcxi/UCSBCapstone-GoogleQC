import assert from 'node:assert/strict';
import test from 'node:test';
import { traceShot } from '../src/model.ts';

function artifacts() {
  return {
    graph: {
      detector_coordinates: [[0, 0, 0], [0, 0, 1], [0, 0, 2]],
      edge_metadata: [{ detectors: [0, 1], observables: [0] }, { detectors: [1, 2], observables: [] }],
      window_annotations: { windows: [
        { id: 'top', layer: 0, commit: [1], buffer: [0, 2] },
        { id: 'left', layer: 1, commit: [0], buffer: [] },
        { id: 'right', layer: 1, commit: [2], buffer: [] },
      ] },
    },
    windows: { windows: [{ window_id: 'top', layer: 0, edge_map: [0, 1] }] },
  };
}

test('both seam directions belong to the middle commit detector', () => {
  for (const [edge, defects, predicted] of [[0, [0, 1], [0]], [1, [1, 2], []]]) {
    const trace = traceShot(artifacts(), { defects, committed_edges: [edge], predicted_observables: predicted, residual_syndrome: [] });
    assert.deepEqual(trace.topEdges, [edge]);
    assert.deepEqual(trace.bottomEdges, []);
    assert.deepEqual(trace.topResidual, []);
  }
});

test('UI rejects residuals inside top commits before attributing layer 1', () => {
  assert.throws(() => traceShot(artifacts(), {
    defects: [1], committed_edges: [], predicted_observables: [], residual_syndrome: [1],
  }), /Residual inside top commit/);
});
