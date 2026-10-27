import tinyCircuit from '../../examples/tiny.stim?raw';
import tinyPlan from '../../examples/tiny-windows.json?raw';
import seamDem from '../../examples/seam.dem?raw';
import seamPlan from '../../examples/windows.json?raw';
import seamShots from '../../examples/seam-shots.json?raw';
import type { Artifacts, Bundle } from './model';

export type ScenarioId = 'seam' | 'tiny';

export const scenarios = {
  seam: {
    id: 'seam', name: 'Seam study', secondary: 'Supplied shots', source: 'Stim DEM',
    sourceFile: 'examples/seam.dem', planFile: 'examples/windows.json',
    sourceText: seamDem, planText: seamPlan, shotsText: seamShots,
    description: 'Follow a defect pair across a top-layer commit and a closed bottom-layer seam.',
    command: 'cargo run --locked -- compile examples/seam.dem --kind dem \\\n  --windows examples/windows.json \\\n  --shots examples/seam-shots.json \\\n  --output artifacts/seam',
    outputPrefix: 'artifacts/seam',
  },
  tiny: {
    id: 'tiny', name: 'Stim circuit', secondary: '24 seeded shots', source: 'Stim circuit',
    sourceFile: 'examples/tiny.stim', planFile: 'examples/tiny-windows.json',
    sourceText: tinyCircuit, planText: tinyPlan, shotsText: null,
    description: 'Start from a noisy repeated measurement, sample Stim, and decode in one CLI invocation.',
    command: 'cargo run --locked -- compile examples/tiny.stim \\\n  --windows examples/tiny-windows.json \\\n  --sample-shots 24 --seed 7 \\\n  --output artifacts/tiny',
    outputPrefix: 'artifacts/tiny',
  },
} as const;

export const STAGES = [
  { short: 'Source', title: 'A circuit becomes evidence', note: 'Stim circuit or DEM', eyebrow: '01 / SOURCE' },
  { short: 'Normalize', title: 'Errors become detector events', note: 'Stim → versioned IR', eyebrow: '02 / STIM BRIDGE' },
  { short: 'Cut graphs', title: 'One graph, two views', note: 'Rust graph + windows', eyebrow: '03 / GRAPH CUTTING' },
  { short: 'Layer 0', title: 'Keep the core. Open the edge.', note: 'Top · open boundary', eyebrow: '04 / FIRST PASS' },
  { short: 'Layer 1', title: 'Close the seam.', note: 'Bottom · closed boundary', eyebrow: '05 / SECOND PASS' },
  { short: 'Verify', title: 'Only verified corrections leave.', note: 'Global parity + residual', eyebrow: '06 / OUTPUT' },
] as const;

export async function loadCapture(id: ScenarioId): Promise<Bundle> {
  const suffixes = ['ir.json', 'graph.json', 'windows.json', 'corrections.json', 'syndromes'] as const;
  const results = await Promise.all(suffixes.map(async suffix => {
    const response = await fetch(`${import.meta.env.BASE_URL}fixtures/${id}.${suffix}`);
    if (!response.ok) throw new Error(`Missing CLI capture: ${id}.${suffix} (HTTP ${response.status})`);
    const text = await response.text();
    return suffix === 'syndromes' ? text : JSON.parse(text);
  }));
  const [ir, graph, windows, corrections, syndromes] = results;
  return { mode: 'capture', artifacts: { ir, graph, windows, corrections, syndromes } as Artifacts };
}
