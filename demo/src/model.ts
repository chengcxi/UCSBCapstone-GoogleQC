export interface EdgeMetadata {
  detectors: number[];
  observables: number[];
  probability: number;
}

export interface WindowPlan {
  id: string;
  layer: number;
  commit: number[];
  buffer: number[];
  temporal_boundary?: string;
}

export interface Initializer {
  vertex_num: number;
  weighted_edges: [number, number, number][];
  virtual_vertices: number[];
}

export interface Graph {
  num_detectors: number;
  num_observables: number;
  detector_coordinates: number[][];
  initializer: Initializer;
  edge_metadata: EdgeMetadata[];
  window_annotations: { windows: WindowPlan[] };
}

export interface WindowGraph {
  window_id: string;
  layer: number;
  temporal_boundary: string;
  initializer: Initializer;
  detector_map: (number | null)[];
  edge_map: number[];
}

export interface Shot {
  defects: number[];
  committed_edges: number[];
  predicted_observables: number[];
  residual_syndrome: number[];
  actual_observables?: number[];
  logical_error?: boolean;
}

export interface Artifacts {
  ir: Record<string, unknown>;
  graph: Graph;
  windows: { schema_version: number; windows: WindowGraph[] };
  corrections: { schema_version: number; shots: Shot[]; num_detectors: number; num_observables: number };
  syndromes: string;
}

export interface Bundle {
  mode: 'capture' | 'live';
  durationMs?: number;
  artifacts: Artifacts;
}

export interface ShotTrace {
  topEdges: number[];
  bottomEdges: number[];
  topResidual: number[];
  finalResidual: number[];
  predicted: number[];
}

export function parityToggle(start: number[], supports: number[][]): number[] {
  const parity = new Set(start);
  for (const support of supports) {
    for (const id of support) {
      if (parity.has(id)) parity.delete(id);
      else parity.add(id);
    }
  }
  return [...parity].sort((a, b) => a - b);
}

/** Attribute committed corrections by incident-detector ownership. */
export function traceShot(artifacts: Artifacts, shot: Shot): ShotTrace {
  const { graph, windows } = artifacts;
  const times = graph.detector_coordinates.map(coords => coords[2]);
  if (times.some(time => !Number.isFinite(time))) throw new Error('Missing detector time coordinate');
  const topOwned = new Set<number>();
  const plans = graph.window_annotations.windows;

  for (const plan of plans.filter(window => window.layer === 0)) {
    const cut = windows.windows.find(window => window.window_id === plan.id);
    if (!cut) throw new Error(`Missing cut graph for ${plan.id}`);
    const members = new Set([...plan.commit, ...plan.buffer]);
    const commit = new Set(plan.commit);
    for (const edgeId of cut.edge_map) {
      const edge = graph.edge_metadata[edgeId];
      if (!edge.detectors.every(id => members.has(id))) {
        if (edge.detectors.some(id => commit.has(id))) throw new Error(`Insufficient buffer for ${plan.id}, edge ${edgeId}`);
        continue;
      }
      if (edge.detectors.some(id => commit.has(id))) {
        if (topOwned.has(edgeId)) throw new Error(`Multiple top windows claim edge ${edgeId}`);
        topOwned.add(edgeId);
      }
    }
  }

  const bottomAvailable = new Set(windows.windows.filter(window => window.layer === 1).flatMap(window => window.edge_map));
  if ([...bottomAvailable].some(edge => topOwned.has(edge))) {
    throw new Error('Overlapping top and bottom edge ownership');
  }
  const topEdges = shot.committed_edges.filter(edge => topOwned.has(edge));
  const bottomEdges = shot.committed_edges.filter(edge => !topOwned.has(edge));
  if (bottomEdges.some(edge => !bottomAvailable.has(edge))) {
    throw new Error('Cannot attribute a correction edge to either decoder layer');
  }
  const topResidual = parityToggle(shot.defects, topEdges.map(id => graph.edge_metadata[id].detectors));
  const topCommit = new Set(plans.filter(w => w.layer === 0).flatMap(w => w.commit));
  const bottomCoverage = new Set(plans.filter(w => w.layer === 1).flatMap(w => w.commit));
  if (topResidual.some(d => topCommit.has(d))) throw new Error('Residual inside top commit after layer 0');
  if (topResidual.some(d => !bottomCoverage.has(d))) throw new Error('Residual outside layer-1 coverage');
  const finalResidual = parityToggle(topResidual, bottomEdges.map(id => graph.edge_metadata[id].detectors));
  const predicted = parityToggle([], shot.committed_edges.map(id => graph.edge_metadata[id].observables));
  if (JSON.stringify(finalResidual) !== JSON.stringify(shot.residual_syndrome)
      || JSON.stringify(predicted) !== JSON.stringify(shot.predicted_observables)) {
    throw new Error('Saved correction does not agree with global syndrome or observable parity');
  }
  return { topEdges, bottomEdges, topResidual, finalResidual, predicted };
}

export function detectorList(ids: number[]): string {
  return ids.length ? ids.map(id => `D${id}`).join(', ') : '∅';
}
export function edgeList(ids: number[]): string {
  return ids.length ? ids.map(id => `e${id}`).join(', ') : '∅';
}
export function observableList(ids: number[]): string {
  return ids.length ? ids.map(id => `L${id}`).join(', ') : '∅';
}
