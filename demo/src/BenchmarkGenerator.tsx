import { useEffect, useState } from 'react';
import { parseGenerationConfig, type GenerationConfig } from './generationConfig';
import { parseGraphComparison, comparisonAccuracy, type GraphComparison } from './graphComparison';

export async function generateCpuDataset(config: GenerationConfig): Promise<GraphComparison> {
  const response = await fetch('/api/generate', { method: 'POST', headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(parseGenerationConfig({ ...config, measure: true })) });
  const value = await response.json();
  if (!response.ok) throw new Error(value.error || 'Generation failed');
  const capture = parseGraphComparison(value);
  if (!capture.cpu_benchmark) {
    const result = comparisonAccuracy(capture.shots);
    throw new Error(`No CPU timing report: ${result.rejected}/${result.total} shots rejected. ${capture.cpu_benchmark_error || 'The CPU comparison could not complete.'}`);
  }
  return capture;
}

export function BenchmarkGenerator({ warmup, repeats, running, onStart, onComplete, onFailure }: {
  warmup: number; repeats: number; running: boolean; onStart: () => void;
  onComplete: (capture: GraphComparison, name: string, imported: boolean) => void; onFailure: (message: string) => void;
}) {
  const [config, setConfig] = useState<GenerationConfig>({ distance: 3, rounds: 6, shots: 16, noise: 0.003,
    seed: 7, basis: 'z', commit: 3, buffer: 3, measure: true, warmup, repeats });
  const [status, setStatus] = useState<{ generationAvailable: boolean; generationReason?: string } | null>(null);
  useEffect(() => {
    fetch('/api/status').then(r => r.json()).then(setStatus)
      .catch(() => setStatus({ generationAvailable: false, generationReason: 'Start the local Vite server to generate and benchmark circuits.' }));
  }, []);
  async function generate() {
    try {
      const settings = parseGenerationConfig({ ...config, warmup, repeats });
      onStart();
      const capture = await generateCpuDataset(settings);
      onComplete(capture, `Generated d${config.distance} · ${config.rounds} rounds · ${config.shots} shots · ${config.basis.toUpperCase()}`, false);
    } catch (error) { onFailure((error as Error).message); }
  }
  async function importFile(file?: File) {
    if (!file) return;
    try {
      if (file.size > 20 * 1024 * 1024) throw new Error('Dataset files must be smaller than 20 MiB');
      const capture = parseGraphComparison(JSON.parse(await file.text()));
      if (!capture.cpu_benchmark) throw new Error('This dataset has no CPU timing report. Import a dataset exported after a successful timing run.');
      onComplete(capture, file.name.replace(/\.json$/i, ''), true);
    } catch (error) { onFailure((error as Error).message); }
  }
  const field = (key: 'distance'|'rounds'|'shots'|'noise'|'seed'|'commit'|'buffer', label: string, min: number, max: number, step = 1) =>
    <div className="bench-field"><label htmlFor={`cpu-generate-${key}`}>{label}</label><input id={`cpu-generate-${key}`} type="number" min={min} max={max} step={step}
      value={config[key]} disabled={running} onChange={e => setConfig(c => ({...c, [key]: Number(e.target.value)}))} /></div>;
  return <section className="bench-panel" aria-label="Generate a CPU benchmark dataset">
    <div className="bench-panel-header"><strong>Generate a benchmark dataset</strong><span>Stim → graph → full-graph and window CPU timings</span></div>
    <div className="generation-form">
      {field('distance', 'Code distance', 3, 7, 2)}{field('rounds', 'Measurement rounds', 2, 90)}
      {field('shots', 'Sampled shots', 1, 256)}{field('noise', 'Noise probability / channel', 0.00001, 0.02, 0.001)}
    </div>
    <details className="comparison-details"><summary>Seed and window settings</summary><div className="generation-form">
      {field('seed', 'Stim seed', 0, 4294967295)}{field('commit', 'Commit width (time slices)', 1, 15)}{field('buffer', 'Buffer width / side', 1, 15)}
      <div className="bench-field"><label htmlFor="cpu-generate-basis">Memory basis</label><select id="cpu-generate-basis" value={config.basis} disabled={running} onChange={e => setConfig(c => ({...c, basis: e.target.value as 'x'|'z'}))}><option value="z">Rotated surface code · Z</option><option value="x">Rotated surface code · X</option></select></div>
    </div></details>
    <div className="generation-actions"><button className="bench-run" disabled={running || !status?.generationAvailable} onClick={() => { void generate(); }}>{running ? 'Running…' : 'Generate and benchmark'}</button>
      <label className="bench-file-button">Import benchmark dataset JSON<input disabled={running} type="file" accept=".json,application/json" onChange={e => { void importFile(e.target.files?.[0]); e.target.value = ''; }} /></label>
      <span>Uses the warmup and measured passes above. Results appear below.</span></div>
    <p className="comparison-note">{status?.generationAvailable ? 'Real local CPU execution. Invalid corrections stop the comparison; rejected shots are never silently filtered.' : status?.generationReason || 'Checking the local generator…'}</p>
    <details className="comparison-details"><summary>Generator setup and limits</summary><pre className="generation-setup">{`.venv/bin/python -m pip install -r requirements-dev.txt\ncargo build --release --locked\ncargo build --release --locked --example window_accuracy`}</pre><p className="comparison-note">Restart Vite after setup. Up to 1,500 global detectors, 400 detectors per window and 256 shots. Correlated DEM components are explicitly approximated as independent edges.</p></details>
  </section>;
}
