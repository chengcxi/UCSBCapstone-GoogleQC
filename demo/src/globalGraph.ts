import type { Artifacts, Shot, ShotTrace, WindowPlan } from './model';

export interface GlobalVertex {
  id: number;
  kind: 'detector' | 'physical';
  time: number | null;
}

export interface GlobalEdge {
  id: number;
  from: number;
  to: number;
  weight: number;
  detectors: number[];
  observables: number[];
  /** This global edge is rewired to a virtual T only in the top local cut. */
  topLocalTemporal: boolean;
  committedIn: 0 | 1 | null;
}

export interface GlobalGraphView {
  vertices: GlobalVertex[];
  edges: GlobalEdge[];
  windows: WindowPlan[];
  syndrome: number[];
  syndromeLabel: string;
}

/** Project the actual *global* Rust graph. Windows annotate its existing IDs;
 * they do not create another copy of the detectors or change global edges.
 * Only exported committed corrections are colored, never tentative MWPM paths. */
export function globalGraphView(artifacts: Artifacts, shot: Shot, trace: ShotTrace, stage: number): GlobalGraphView {
  const { graph, windows } = artifacts;
  if (graph.initializer.vertex_num < graph.num_detectors
      || graph.initializer.weighted_edges.length !== graph.edge_metadata.length) {
    throw new Error('Inconsistent global graph artifact');
  }
  const virtual = new Set(graph.initializer.virtual_vertices);
  if (Array.from({ length: graph.initializer.vertex_num }, (_, id) => id)
      .some(id => virtual.has(id) !== (id >= graph.num_detectors))) {
    throw new Error('Inconsistent global virtual boundary vertices');
  }
  const topLocalTemporal = new Set<number>();
  for (const window of windows.windows.filter(item => item.layer === 0)) {
    window.initializer.weighted_edges.forEach(([left, right], index) => {
      const edgeId = window.edge_map[index];
      // A two-detector global edge ending at a null local detector_map entry
      // was cut at an artificial temporal boundary. The global edge stays whole.
      if (graph.edge_metadata[edgeId]?.detectors.length === 2
          && (window.detector_map[left] === null || window.detector_map[right] === null)) {
        topLocalTemporal.add(edgeId);
      }
    });
  }
  const visibleTop = stage >= 3 ? new Set(trace.topEdges) : new Set<number>();
  const visibleBottom = stage >= 4 ? new Set(trace.bottomEdges) : new Set<number>();

  return {
    vertices: Array.from({ length: graph.initializer.vertex_num }, (_, id) => ({
      id,
      kind: virtual.has(id) ? 'physical' : 'detector',
      time: id < graph.num_detectors ? graph.detector_coordinates[id]?.[2] ?? null : null,
    })),
    edges: graph.initializer.weighted_edges.map(([from, to, weight], id) => ({
      id, from, to, weight,
      detectors: graph.edge_metadata[id].detectors,
      observables: graph.edge_metadata[id].observables,
      topLocalTemporal: topLocalTemporal.has(id),
      committedIn: visibleTop.has(id) ? 0 : visibleBottom.has(id) ? 1 : null,
    })),
    windows: graph.window_annotations.windows,
    syndrome: stage < 3 ? shot.defects : stage === 3 ? trace.topResidual : trace.finalResidual,
    syndromeLabel: stage < 3 ? 'SELECTED SHOT · S₀' : stage === 3 ? 'AFTER TOP COMMIT · S₁' : 'AFTER BOTTOM COMMIT · S₂',
  };
}
