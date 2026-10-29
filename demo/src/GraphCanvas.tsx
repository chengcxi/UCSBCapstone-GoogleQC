import { globalGraphView, type GlobalGraphView } from './globalGraph';
import type { Artifacts, Shot, ShotTrace, WindowPlan } from './model';
import { detectorList, edgeList } from './model';

type Point = { x: number; y: number };
type Props = { artifacts: Artifacts; shot: Shot; trace: ShotTrace; stage: number };

/** Detector time is the horizontal axis. The geometry is schematic, but
 * every vertex, edge, weight, ID and window membership comes from Rust. */
function layout(view: GlobalGraphView): Point[] {
  const detectors = view.vertices.filter(vertex => vertex.kind === 'detector');
  const times = [...new Set(detectors.map(vertex => vertex.time ?? vertex.id))].sort((a, b) => a - b);
  const atTime = (time: number) => detectors.filter(vertex => (vertex.time ?? vertex.id) === time);
  const points: Point[] = Array.from({ length: view.vertices.length }, () => ({ x: 360, y: 185 }));
  for (const vertex of detectors) {
    const time = vertex.time ?? vertex.id;
    const column = times.indexOf(time);
    const peers = atTime(time);
    const slot = peers.findIndex(peer => peer.id === vertex.id);
    points[vertex.id] = {
      x: (times.length === 1 ? 360 : 134 + 452 * column / (times.length - 1)) + (slot - (peers.length - 1) / 2) * 44,
      y: 185,
    };
  }
  for (const vertex of view.vertices.filter(item => item.kind === 'physical')) {
    const neighbors = view.edges.flatMap(edge => edge.from === vertex.id ? [edge.to] : edge.to === vertex.id ? [edge.from] : [])
      .filter(id => id < points.length && view.vertices[id].kind === 'detector');
    const mean = neighbors.length ? neighbors.reduce((sum, id) => sum + points[id].x, 0) / neighbors.length : 360;
    const lastDetector = Math.max(...detectors.map(item => item.id));
    const firstDetector = Math.min(...detectors.map(item => item.id));
    const side = neighbors.length === 1 ? (neighbors[0] === lastDetector ? 76 : neighbors[0] === firstDetector ? -76 : 0) : 0;
    points[vertex.id] = { x: Math.max(64, Math.min(690, mean + side)), y: 260 + (vertex.id - detectors.length) * 24 };
  }
  return points;
}

export function GraphCanvas({ artifacts, shot, trace, stage }: Props) {
  const view = globalGraphView(artifacts, shot, trace, stage);
  const points = layout(view);
  const current = new Set(view.syndrome);
  const top = view.windows.filter(window => window.layer === 0);
  const bottom = view.windows.filter(window => window.layer === 1);
  if (!top.length || !bottom.length) return <div className="canvas-empty">The two window annotations are missing.</div>;

  function drawWindowBand(window: WindowPlan) {
    const members = [...window.commit, ...window.buffer].filter(id => points[id]);
    if (!members.length) return null;
    const min = Math.min(...members.map(id => points[id].x));
    const max = Math.max(...members.map(id => points[id].x));
    const width = Math.max(206, max - min + 108);
    const x = Math.max(34, Math.min(726 - width, (min + max - width) / 2));
    const y = window.layer === 0 ? 44 : 320;
    const inactive = stage < 2 || (stage === 3 && window.layer === 1);
    const name = `LAYER ${window.layer} · ${(window.temporal_boundary ?? 'CUT').toUpperCase()}`;
    return (
      <g key={window.id} className={`global-window-band band-${window.layer === 0 ? 'top' : 'bottom'} ${inactive ? 'band-idle' : ''}`}>
        <rect className="window-band-fill" x={x} y={y} width={width} height="67" rx="12" />
        <text className="window-band-title" x={x + 14} y={y + 22}>{name}</text>
        <text className="window-band-id" x={x + width - 14} y={y + 22} textAnchor="end">{window.id}</text>
        {members.map(id => {
          const commit = window.commit.includes(id);
          return (
            <g key={`${window.id}-${id}`}>
              <title>{`${window.id}: D${id} is ${commit ? 'committed' : 'buffered'}; same D${id} in the global graph`}</title>
              <line className="window-guide" x1={points[id].x} x2={points[id].x}
                y1={window.layer === 0 ? y + 67 : 207} y2={window.layer === 0 ? 162 : y} />
              <rect className={`window-member ${commit ? 'member-commit' : 'member-buffer'}`} x={points[id].x - 52} y={y + 35} width="104" height="23" rx="5" />
              <text className={`window-member-text ${commit ? 'member-commit-text' : 'member-buffer-text'}`}
                x={points[id].x} y={y + 50} textAnchor="middle">D{id} {commit ? 'COMMIT' : 'BUFFER'}</text>
            </g>
          );
        })}
      </g>
    );
  }

  return (
    <div className="graph-visual" role="img" aria-label={`One global detector graph with ${artifacts.graph.num_detectors} unique detectors and ${view.edges.length} weighted edges. Top open window commits ${detectorList(top.flatMap(window => window.commit))} and buffers ${detectorList(top.flatMap(window => window.buffer))}; bottom closed window commits ${detectorList(bottom.flatMap(window => window.commit))}. Input defects ${detectorList(shot.defects)}. Top committed ${edgeList(trace.topEdges)}; bottom committed ${edgeList(trace.bottomEdges)}. Current syndrome ${detectorList(view.syndrome)}.`}>
      <svg viewBox="0 0 760 405" preserveAspectRatio="xMidYMid meet" className="graph-svg">
        <defs>
          <pattern id="graph-dots" width="24" height="24" patternUnits="userSpaceOnUse"><circle cx="1" cy="1" r="1" fill="#d8e5df" /></pattern>
        </defs>
        <rect width="760" height="405" fill="url(#graph-dots)" opacity=".6" />
        <text x="44" y="25" className="axis-title global-axis-title">ONE GLOBAL GRAPH</text>
        <text x="716" y="25" textAnchor="end" className="axis-title">{artifacts.graph.num_detectors} DETECTORS · {view.edges.length} EDGES</text>
        {top.map(drawWindowBand)}
        {bottom.map(drawWindowBand)}
        <text x="44" y="291" className="syndrome-readout">{view.syndromeLabel} <tspan className="syndrome-value">{detectorList(view.syndrome)}</tspan></text>
        <text x="716" y="139" textAnchor="end" className="axis-title">DETECTOR TIME →</text>
        {view.edges.map(edge => {
          const a = points[edge.from];
          const b = points[edge.to];
          const physical = view.vertices[edge.from].kind === 'physical' || view.vertices[edge.to].kind === 'physical';
          // Offset physical-boundary tags away from the detector's time label.
          const labelX = (a.x + b.x) / 2 + (physical ? 20 : 0);
          const labelY = physical ? (a.y + b.y) / 2 - 5 : Math.min(a.y, b.y) - 38;
          const state = edge.committedIn === 0 ? 'global-edge-top' : edge.committedIn === 1 ? 'global-edge-bottom' : 'global-edge-idle';
          return (
            <g key={edge.id} data-global-edge={edge.id} data-endpoints={`${edge.from},${edge.to}`}
              className={`global-edge ${state}`}>
              <title>{`Global e${edge.id}: ${detectorList(edge.detectors)} → ${edge.observables.length ? `L${edge.observables.join(', L')}` : 'no logical flip'}; weight ${edge.weight}${edge.topLocalTemporal ? '. Rewired to T in the top local cut only.' : ''}${edge.committedIn !== null ? `. Committed in layer ${edge.committedIn}.` : ''}`}</title>
              <line className="global-edge-line" x1={a.x} y1={a.y} x2={b.x} y2={b.y} />
              <rect className="global-edge-tag" x={labelX - 36} y={labelY - 13} width="72" height="26" rx="6" />
              <text className="global-edge-label" x={labelX} y={labelY + 4} textAnchor="middle">e{edge.id}<tspan className="weight-tag"> /{edge.weight}</tspan></text>
            </g>
          );
        })}
        {stage >= 2 && view.edges.filter(edge => edge.topLocalTemporal).map(edge => {
          const x = (points[edge.from].x + points[edge.to].x) / 2;
          return <g key={`cut-${edge.id}`} className="local-cut-marker">
            <title>{`In the top cut window only, e${edge.id} is rewired to the artificial temporal boundary T. The global e${edge.id} remains whole.`}</title>
            <line x1={x} x2={x} y1="209" y2="247" className="local-cut-guide" />
            <rect x={x - 70} y="247" width="140" height="24" rx="5" className="local-cut-tag" />
            <text x={x} y="263" textAnchor="middle" className="local-cut-text">e{edge.id} → T IN TOP CUT</text>
          </g>;
        })}
        {view.vertices.map(vertex => {
          const { x, y } = points[vertex.id];
          const defect = vertex.kind === 'detector' && current.has(vertex.id);
          const topCore = top.some(window => window.commit.includes(vertex.id));
          const bottomCore = bottom.some(window => window.commit.includes(vertex.id));
          const halo = stage === 3 && topCore ? 'node-top-halo' : stage >= 4 && bottomCore ? 'node-bottom-halo' : '';
          return (
            <g key={vertex.id} data-global-vertex={vertex.id} className={`global-node node-${vertex.kind} ${defect ? 'node-defect' : ''} ${halo}`}>
              <title>{vertex.kind === 'detector' ? `Global detector D${vertex.id} · t=${vertex.time ?? '?'}${defect ? ' · current syndrome defect' : ''}` : `Global physical boundary V${vertex.id - artifacts.graph.num_detectors}`}</title>
              {halo && <circle cx={x} cy={y} r="29" className="node-halo" />}
              {vertex.kind === 'physical' ? (
                <rect className="node-shape" x={x - 17} y={y - 17} width="34" height="34" rx="7" />
              ) : <circle className="node-shape" cx={x} cy={y} r="20" />}
              <text className="node-label" x={x} y={y + 5} textAnchor="middle">{vertex.kind === 'detector' ? `D${vertex.id}` : 'V'}</text>
              <text className="node-caption" x={x} y={y + 43} textAnchor="middle">
                {vertex.kind === 'detector' ? `t = ${vertex.time ?? '?'}` : 'PHYSICAL'}
              </text>
            </g>
          );
        })}
      </svg>
      <div className="graph-pan-hint" aria-hidden="true">← SWIPE TO EXPLORE →</div>
      <div className="graph-key">
        <span><i className="key-dot key-defect" /> current syndrome</span>
        <span><i className="key-line key-top" /> top commit</span>
        <span><i className="key-line key-bottom" /> bottom commit</span>
        <span><i className="key-ring" /> global physical boundary</span>
      </div>
      <div className="graph-note">One graph, one D per detector. Window bands reuse those IDs; an artificial T is never a global vertex.</div>
    </div>
  );
}
