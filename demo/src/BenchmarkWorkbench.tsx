import { useEffect, useRef, useState } from 'react';
import { ArrowDownToLine, Clipboard, FlaskConical, Play } from 'lucide-react';
import { scenarios, type ScenarioId } from './data';
import { accuracyLabel, latencyLabel, methodAccuracy, parseBenchmarkReport, type BenchmarkReport, type LatencyStats } from './benchmarkReport';
import { detectorList, edgeList, observableList, type Graph } from './model';
import { BenchmarkGenerator, generateCpuDataset } from './BenchmarkGenerator';
import { savedDatasets, saveDataset, type SavedDataset } from './datasetStore';
import { parseGraphComparison, type GraphComparison } from './graphComparison';

type Props = {
  scenarioId: ScenarioId;
  onScenarioChange: (scenario: ScenarioId) => void;
  sourceHash?: string;
  graph?: Graph;
  binaryPresent: boolean;
  unavailableReason: string | null;
};

function cpuCommand(scenario: ScenarioId, warmup: number, repeats: number): string {
  const prefix = 'cargo run --release --locked -- benchmark';
  const settings = `--warmup ${warmup} --repeats ${repeats}`;
  if (scenario === 'seam') {
    return `${prefix} demo/public/fixtures/seam.ir.json \\\n  --shots examples/seam-shots.json \\\n  ${settings} --output artifacts/seam.benchmark.json`;
  }
  return `mkdir -p artifacts\nnode demo/scripts/benchmark-shots.mjs \\\n  demo/public/fixtures/tiny.corrections.json artifacts/tiny-benchmark-shots.json\n${prefix} demo/public/fixtures/tiny.ir.json \\\n  --shots artifacts/tiny-benchmark-shots.json \\\n  ${settings} --output artifacts/tiny.benchmark.json`;
}

function correctness(actual: number[] | null, predicted: number[]) {
  if (actual === null) return { label: 'N/A', className: 'shot-unknown' };
  const correct = JSON.stringify(actual) === JSON.stringify(predicted);
  return { label: correct ? 'correct' : 'ERROR', className: correct ? 'shot-correct' : 'shot-error' };
}

function LatencyPlot({ global, windows }: { global: LatencyStats; windows: LatencyStats }) {
  const limit = Math.max(global.median_ns, global.p95_ns, windows.median_ns, windows.p95_ns, 1) * 1.08;
  const unit = limit >= 1_000_000 ? 'ms' : limit >= 1_000 ? 'µs' : 'ns';
  const divisor = unit === 'ms' ? 1_000_000 : unit === 'µs' ? 1_000 : 1;
  const x = (ns: number) => 138 + ns / limit * 516;
  const axis = Array.from({ length: 5 }, (_, index) => 138 + index * 129);
  const methods = [
    { name: 'GLOBAL CPU', stats: global, y: 79, cls: 'baseline' },
    { name: 'WINDOW CPU', stats: windows, y: 153, cls: 'windowed' },
  ];
  return <div className="benchmark-plot-scroll" tabIndex={0} aria-label="Scrollable latency chart">
    <svg className="benchmark-plot" viewBox="0 0 708 242" role="img"
      aria-label={`Measured CPU latency. Global median ${latencyLabel(global.median_ns)}, p95 ${latencyLabel(global.p95_ns)}. Window median ${latencyLabel(windows.median_ns)}, p95 ${latencyLabel(windows.p95_ns)}.`}>
      <rect x="138" y="29" width="516" height="153" className="plot-field" />
      {axis.map((tick, index) => <g key={index}>
        <line x1={tick} x2={tick} y1="29" y2="182" className="plot-grid" />
        <text x={tick} y="202" textAnchor="middle" className="plot-tick">{((limit * index / 4) / divisor).toFixed(unit === 'ns' ? 0 : 2)}</text>
      </g>)}
      {methods.map(method => <g key={method.cls} className={`plot-method method-${method.cls}`}>
        <text x="129" y={method.y + 4} textAnchor="end" className="plot-row-label">{method.name}</text>
        <rect x="138" y={method.y - 13} width={Math.max(1, x(method.stats.median_ns) - 138)} height="26" className="plot-median-bar" />
        <line x1={x(method.stats.p95_ns)} x2={x(method.stats.p95_ns)} y1={method.y - 21} y2={method.y + 21} className="plot-p95-mark" />
        <text x={Math.min(x(method.stats.median_ns) + 8, 640)} y={method.y + 4} className="plot-median-value">{latencyLabel(method.stats.median_ns)}</text>
      </g>)}
      <text x="396" y="231" textAnchor="middle" className="plot-axis-label">LATENCY PER SHOT ({unit})  ·  BAR: MEDIAN  ·  TICK: P95</text>
    </svg>
  </div>;
}

export function BenchmarkWorkbench({ scenarioId, onScenarioChange, sourceHash, graph, binaryPresent, unavailableReason }: Props) {
  const [datasets, setDatasets] = useState<SavedDataset[]>([]);
  const [selected, setSelected] = useState<SavedDataset | null>(null);
  const [storageNote, setStorageNote] = useState<string | null>(null);
  const [warmup, setWarmup] = useState(2);
  const [repeats, setRepeats] = useState(20);
  const [report, setReport] = useState<BenchmarkReport | null>(null);
  const [origin, setOrigin] = useState<'local' | 'imported' | 'ci-sample' | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [running, setRunning] = useState(false);
  const [population, setPopulation] = useState<'nonempty' | 'all'>('nonempty');
  const requestEpoch = useRef(0);
  const userMeasurement = useRef(false);
  const activeGraph = selected?.capture.graph ?? graph;
  const activeHash = selected?.capture.cpu_benchmark?.provenance?.input_sha256 ?? sourceHash;

  useEffect(() => {
    savedDatasets().then(rows => setDatasets(rows.filter(row => {
      try { return !!parseGraphComparison(row.capture).cpu_benchmark; } catch { return false; }
    }))).catch(() => setStorageNote('Browser storage is unavailable; export your dataset to keep it.'));
  }, []);

  useEffect(() => {
    const epoch = ++requestEpoch.current;
    userMeasurement.current = false; setSelected(null);
    setReport(null); setOrigin(null); setError(null); setRunning(false);
    // This optional, committed report was measured by the release Rust CLI on
    // GitHub Actions, not inferred from saved corrections. A different dataset
    // starts empty until the user runs or imports its own measurement.
    if (scenarioId !== 'seam' || !sourceHash) return;
    fetch(`${import.meta.env.BASE_URL}fixtures/seam.benchmark.json`)
      .then(response => response.ok ? response.json() : null)
      .then(value => {
        if (!value || epoch !== requestEpoch.current || userMeasurement.current) return;
        const sample = parseBenchmarkReport(value, graph);
        if (sample.provenance?.input_sha256 !== sourceHash || sample.settings.shot_count !== 5) return;
        setReport(sample); setOrigin('ci-sample');
      }).catch(() => { /* The import and local-run tools remain usable without the CI sample. */ });
  }, [scenarioId, sourceHash, graph]);

  function verifySource(next: BenchmarkReport) {
    if (!activeHash || !activeGraph) throw new Error('Load the matching graph dataset before opening a report.');
    if (next.provenance?.input_sha256 !== activeHash) {
      throw new Error('This report was produced from a different source. Select its matching fixture before loading it.');
    }
    if (next.settings.shot_count !== (selected ? selected.capture.shots.length : scenarioId === 'tiny' ? 24 : 5)) {
      throw new Error('This report has a different shot count from the selected example.');
    }
    if (selected && next.shots.some((shot, i) => JSON.stringify(shot.defects) !== JSON.stringify(selected.capture.shots[i].defects)
      || JSON.stringify(shot.actual_observables) !== JSON.stringify(selected.capture.shots[i].actual))) throw new Error('Report shot inputs differ from this dataset.');
  }

  async function acceptGenerated(capture: GraphComparison, name: string, imported: boolean) {
    if (!capture.cpu_benchmark) return;
    userMeasurement.current = true; ++requestEpoch.current;
    const row: SavedDataset = { id: selected && !imported && selected.name === name ? selected.id : crypto.randomUUID(), name, origin: imported ? 'imported' : 'generated', capture };
    setDatasets(previous => [...previous.filter(item => item.id !== row.id), row]);
    setSelected(row); setReport(capture.cpu_benchmark); setOrigin(imported ? 'imported' : 'local'); setError(null); setRunning(false);
    setWarmup(capture.cpu_benchmark.settings.warmup_passes); setRepeats(capture.cpu_benchmark.settings.measured_passes);
    try { await saveDataset(row); } catch { setStorageNote('Dataset is available this session. Export dataset JSON to keep a copy.'); }
  }

  function chooseDataset(value: string) {
    ++requestEpoch.current; userMeasurement.current = true;
    setError(null); setRunning(false);
    const row = datasets.find(item => item.id === value);
    if (row) {
      setSelected(row); setReport(row.capture.cpu_benchmark ?? null); setOrigin('imported');
      if (row.capture.cpu_benchmark) { setWarmup(row.capture.cpu_benchmark.settings.warmup_passes); setRepeats(row.capture.cpu_benchmark.settings.measured_passes); }
    } else {
      setSelected(null); setReport(null); setOrigin(null); onScenarioChange(value as ScenarioId);
      if (value === scenarioId && value === 'seam') {
        const epoch = requestEpoch.current;
        fetch(`${import.meta.env.BASE_URL}fixtures/seam.benchmark.json`).then(r => r.json()).then(value => {
          if (epoch === requestEpoch.current) { setReport(parseBenchmarkReport(value, graph)); setOrigin('ci-sample'); }
        }).catch(() => {});
      }
    }
  }

  async function run() {
    if (!binaryPresent || !Number.isInteger(warmup) || warmup < 0 || warmup > 10
        || !Number.isInteger(repeats) || repeats < 1 || repeats > 100) {
      setError('Build the Rust CLI and use 0–10 warmup passes and 1–100 measured passes.');
      return;
    }
    const epoch = requestEpoch.current;
    setRunning(true); setError(null); setReport(null);
    try {
      if (selected) {
        if (!selected.capture.generation_settings) throw new Error('This imported dataset has no generation settings. Generate a fresh dataset to measure locally.');
        const capture = await generateCpuDataset({ ...selected.capture.generation_settings, warmup, repeats, measure: true });
        if (epoch === requestEpoch.current) await acceptGenerated(capture, selected.name, false);
        return;
      }
      const response = await fetch('/api/benchmark', {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ scenario: scenarioId, warmup, repeats }),
      });
      const result = await response.json();
      if (!response.ok) throw new Error(result.error || 'The CPU benchmark could not complete.');
      const next = parseBenchmarkReport(result, activeGraph);
      verifySource(next);
      if (epoch === requestEpoch.current) { userMeasurement.current = true; setReport(next); setOrigin('local'); }
    } catch (caught) {
      if (epoch === requestEpoch.current) setError((caught as Error).message);
    } finally {
      if (epoch === requestEpoch.current) setRunning(false);
    }
  }

  async function importReport(file?: File) {
    if (!file) return;
    const epoch = requestEpoch.current;
    try {
      const next = parseBenchmarkReport(JSON.parse(await file.text()), activeGraph);
      if (epoch !== requestEpoch.current) return;
      verifySource(next);
      userMeasurement.current = true;
      setReport(next); setOrigin('imported'); setError(null);
    } catch (caught) {
      if (epoch === requestEpoch.current) setError(`Report not loaded: ${(caught as Error).message}`);
    }
  }

  function downloadReport() {
    if (!report) return;
    const url = URL.createObjectURL(new Blob([JSON.stringify(report, null, 2) + '\n'], { type: 'application/json' }));
    const link = document.createElement('a');
    link.href = url; link.download = `${selected ? 'generated' : scenarioId}.benchmark.json`; link.click();
    setTimeout(() => URL.revokeObjectURL(url), 0);
  }

  async function copyCommand() {
    try { await navigator.clipboard.writeText(command); }
    catch { setError('Clipboard access is unavailable. The command is also shown below.'); }
  }

  const useNonempty = population === 'nonempty' && (report?.settings.nonempty_shot_count ?? 0) > 0;
  const baseline = useNonempty ? report?.baseline.nonempty_shots : report?.baseline.all_shots;
  const windows = useNonempty ? report?.windowed.nonempty_shots : report?.windowed.all_shots;
  const baselineAccuracy = report && methodAccuracy(report, 'baseline', useNonempty);
  const windowAccuracy = report && methodAccuracy(report, 'windowed', useNonempty);
  const ratio = baseline && windows && windows.median_ns > 0 ? baseline.median_ns / windows.median_ns : null;
  const settings = selected?.capture.generation_settings;
  const command = settings ? `.venv/bin/python tools/benchmark_window.py --distance ${settings.distance} --rounds ${settings.rounds} --shots ${settings.shots} --noise ${settings.noise} --seed ${settings.seed} --basis ${settings.basis} --commit-slices ${settings.commit} --buffer-slices ${settings.buffer} --output artifacts/generated\ncargo run --release --locked -- benchmark artifacts/generated/decoded.ir.json \\\n  --shots artifacts/generated/shots.json --allow-correlated \\\n  --warmup ${warmup} --repeats ${repeats} --output artifacts/generated.benchmark.json` : selected ? 'Import the dataset generation settings to reproduce these inputs locally.' : cpuCommand(scenarioId, warmup, repeats);

  return <section className="benchmark-workbench" aria-label="CPU decoder benchmark">
    <div className="bench-heading">
      <div><div className="micro-label">MEASUREMENT / DECODER COMPARISON</div><h1>CPU benchmark</h1>
        <p>One global Micro Blossom solver versus the two-layer window solver, run sequentially on identical captured shots.</p></div>
      <div className={`bench-status ${binaryPresent ? 'bench-ready' : ''}`}>{binaryPresent ? 'RUST BINARY READY' : report ? 'BUILD TO RERUN' : 'LOCAL BINARY REQUIRED'}</div>
    </div>

    <section className="bench-panel bench-setup" aria-label="Benchmark settings">
      <div className="bench-panel-header"><strong>Configuration</strong><span>Saved and generated datasets · real CPU execution</span></div>
      <div className="bench-form">
        <div className="bench-field"><label htmlFor="benchmark-scenario">Dataset</label>
          <select id="benchmark-scenario" disabled={running} value={selected?.id ?? scenarioId} onChange={event => chooseDataset(event.target.value)}>
            <option value="seam">Seam DEM · 5 supplied shots</option>
            <option value="tiny">Stim circuit · 24 seed-7 shots</option>
            {datasets.map(row => <option value={row.id} key={row.id}>{row.name}</option>)}
          </select></div>
        <div className="bench-field bench-number"><label htmlFor="benchmark-warmup">Warmup passes</label>
          <input id="benchmark-warmup" disabled={running} type="number" min="0" max="10" value={warmup} onChange={event => setWarmup(Number(event.target.value))} /></div>
        <div className="bench-field bench-number"><label htmlFor="benchmark-repeats">Measured passes</label>
          <input id="benchmark-repeats" disabled={running} type="number" min="1" max="100" value={repeats} onChange={event => setRepeats(Number(event.target.value))} /></div>
        <button type="button" className="bench-run" onClick={run} disabled={running || !binaryPresent}>
          <Play size={13} /> {running ? 'Measuring…' : 'Run CPU comparison'}
        </button>
      </div>
      <div className="bench-setup-bottom">
        <span>{selected ? 'Generated datasets use the same seed and shot count when rerun. Stim is required.' : binaryPresent ? 'Python/Stim are not needed: these shots were already captured.' : unavailableReason ?? 'No Rust binary found. Import a real benchmark report or build the CLI locally.'}</span>
        <div><label className="bench-file-button">Load report JSON<input disabled={running} type="file" accept=".json,application/json" onChange={event => { void importReport(event.target.files?.[0]); event.target.value = ''; }} /></label>
          <button type="button" onClick={copyCommand}><Clipboard size={13} /> Copy command</button></div>
      </div>
      {selected && <div className="generation-actions"><button onClick={() => {
        const url = URL.createObjectURL(new Blob([JSON.stringify(selected.capture)], {type:'application/json'}));
        const link = document.createElement('a'); link.href = url; link.download = 'generated-benchmark.dataset.json'; link.click(); setTimeout(() => URL.revokeObjectURL(url), 0);
      }}>Export dataset JSON</button><span>{selected.capture.graph.num_detectors} detectors · {selected.capture.graph.initializer.weighted_edges.length} edges · {selected.capture.windows.length} windows</span></div>}
      {error && <p className="bench-error" role="alert">{error}</p>}
      {storageNote && <p role="status" className="comparison-note">{storageNote}</p>}
    </section>

    <BenchmarkGenerator warmup={warmup} repeats={repeats} running={running}
      onStart={() => { ++requestEpoch.current; userMeasurement.current = true; setReport(null); setError(null); setRunning(true); }}
      onComplete={(capture, name, imported) => { void acceptGenerated(capture, name, imported); }}
      onFailure={message => { setRunning(false); setError(message); }} />

    <div className="bench-results-header"><strong>Results</strong><span>{report ? `${origin === 'local' ? 'MEASURED HERE' : origin === 'ci-sample' ? 'RECORDED CI SAMPLE' : 'IMPORTED REPORT'} · ${report.host.profile.toUpperCase()} · ${report.host.os}/${report.host.arch}` : 'NO TIMING DATA LOADED'}</span></div>
    {report && baseline && windows && baselineAccuracy && windowAccuracy ? <>
      <div className="bench-results-grid">
        <section className="bench-panel bench-plot-panel" aria-label="Latency chart">
          <div className="bench-panel-header"><strong>Latency distribution</strong><div className="bench-tabs" role="group" aria-label="Shot population">
            <button className={useNonempty ? 'selected' : ''} disabled={!report.settings.nonempty_shot_count} onClick={() => setPopulation('nonempty')}>Nonempty ({report.settings.nonempty_shot_count})</button>
            <button className={!useNonempty ? 'selected' : ''} onClick={() => setPopulation('all')}>All ({report.settings.shot_count})</button>
          </div></div>
          <LatencyPlot global={baseline} windows={windows} />
          <div className="bench-plot-note">{report.settings.measured_passes} measured passes × {useNonempty ? report.settings.nonempty_shot_count : report.settings.shot_count} shots per method. Empty syndromes are {useNonempty ? 'excluded from this view' : 'included'}.</div>
        </section>
        <section className="bench-panel bench-summary" aria-label="Measurement summary">
          <div className="bench-panel-header"><strong>Summary · {useNonempty ? 'Nonempty shots' : 'All shots'}</strong><button onClick={downloadReport}><ArrowDownToLine size={13} /> Save JSON</button></div>
          <table><thead><tr><th scope="col">Metric</th><th scope="col">Global CPU</th><th scope="col">Window CPU</th></tr></thead><tbody>
            <tr><th scope="row">Median / shot</th><td>{latencyLabel(baseline.median_ns)}</td><td>{latencyLabel(windows.median_ns)}</td></tr>
            <tr><th scope="row">95th percentile</th><td>{latencyLabel(baseline.p95_ns)}</td><td>{latencyLabel(windows.p95_ns)}</td></tr>
            <tr><th scope="row">Mean / shot</th><td>{latencyLabel(baseline.mean_ns)}</td><td>{latencyLabel(windows.mean_ns)}</td></tr>
            <tr className="bench-accuracy-row"><th scope="row">Logical accuracy</th><td>{accuracyLabel(baselineAccuracy)}</td><td>{accuracyLabel(windowAccuracy)}</td></tr>
            <tr><th scope="row">Logical errors</th><td>{baselineAccuracy.total ? `${baselineAccuracy.total - baselineAccuracy.correct}/${baselineAccuracy.total}` : 'N/A'}</td><td>{windowAccuracy.total ? `${windowAccuracy.total - windowAccuracy.correct}/${windowAccuracy.total}` : 'N/A'}</td></tr>
            <tr><th scope="row">Execution</th><td>CPU, single graph</td><td>CPU, sequential cuts</td></tr>
            <tr className="bench-unmeasured"><th scope="row">FPGA parallel</th><td colSpan={2}>NOT MEASURED · NOT IMPLEMENTED</td></tr>
          </tbody></table>
          <div className="bench-summary-foot">{ratio === null ? 'Median ratio unavailable for zero-duration samples.' : `Observed global / window median: ${ratio.toFixed(2)}× (CPU only; values below 1 mean the global path was faster, not an FPGA projection).`}<br />Logical accuracy is an exact match of all predicted observable flips to actual Stim/supplied flips, counted once per truth-labeled input shot in the selected {useNonempty ? 'nonempty' : 'all-shot'} population—not once per timing repeat. N/A means no ground truth. Clearing the global syndrome is verified separately; five supplied shots do not establish a general error rate.</div>
        </section>
      </div>
      <section className="bench-panel bench-shot-panel" aria-label="Per-shot comparison">
        <div className="bench-panel-header"><strong>Shot-level verification</strong><span>{report.prediction_disagreements} observable prediction disagreements · {report.graph.detectors} detectors / {report.graph.edges} edges</span></div>
        <div className="bench-mobile-hint">Swipe table horizontally to inspect both corrections →</div>
        <div className="bench-table-scroll" tabIndex={0} aria-label="Scrollable shot comparison"><table><thead><tr><th scope="col">Shot</th><th scope="col">Input S</th><th scope="col">Actual L</th><th scope="col">Global correction</th><th scope="col">Global L</th><th scope="col">Global correct?</th><th scope="col">Window correction</th><th scope="col">Window L</th><th scope="col">Window correct?</th><th scope="col">Pred. match</th></tr></thead><tbody>
          {report.shots.map(shot => {
            const globalResult = correctness(shot.actual_observables, shot.baseline_observables);
            const windowResult = correctness(shot.actual_observables, shot.window_observables);
            return <tr key={shot.index}><td>{String(shot.index + 1).padStart(2, '0')}</td><td>{detectorList(shot.defects)}</td><td>{shot.actual_observables === null ? 'unknown' : observableList(shot.actual_observables)}</td><td>{edgeList(shot.baseline_edges)}</td><td>{observableList(shot.baseline_observables)}</td><td className={globalResult.className}>{globalResult.label}</td><td>{edgeList(shot.window_edges)}</td><td>{observableList(shot.window_observables)}</td><td className={windowResult.className}>{windowResult.label}</td><td>{shot.same_prediction ? 'agree' : 'DIFFER'}</td></tr>;
          })}
        </tbody></table></div>
      </section>
      <div className="bench-provenance">Source SHA-256: {report.provenance?.input_sha256 ?? 'not provided'} · CPU profile: {report.host.profile} · {report.settings.order}. {origin === 'ci-sample' ? <>
        Actual release-binary measurement on a shared GitHub Actions runner, <a href="https://github.com/chengcxi/microblossom-parallelism/actions/runs/40595098673" target="_blank" rel="noreferrer">run 40595098673</a> (commit c486703). Example-scale timing is not a hardware performance claim; rerun locally.
      </> : origin === 'imported' ? 'Imported timings are user-provided; the browser verifies structure, statistics and global correction parity, but cannot independently attest to timing provenance.' : 'Measured by the local Rust binary; shared-host jitter may affect the result.'}</div>
    </> : <section className="bench-panel bench-empty" aria-label="No benchmark measurements">
      <FlaskConical size={25} strokeWidth={1.4} /><div><strong>No measurements available</strong>
        <p>Run the Rust benchmark on this machine, or load a report generated by the CLI. No times, speedups, or FPGA results are estimated from correction artifacts.</p></div>
      <div className="bench-empty-table"><span>METRIC</span><span>GLOBAL CPU</span><span>WINDOW CPU</span><span>FPGA</span>
        <strong>Median / shot</strong><b>—</b><b>—</b><b>NOT MEASURED</b>
        <strong>Logical accuracy</strong><b>—</b><b>—</b><b>NOT MEASURED</b></div>
    </section>}

    <div className="bench-bottom-grid">
      <section className="bench-panel bench-methodology"><div className="bench-panel-header"><strong>Measurement protocol</strong><span>REAL EXECUTION ONLY</span></div>
        <ol><li>Both decoders receive the same {selected ? 'seeded Stim samples' : scenarios[scenarioId].secondary.toLowerCase()} and the same weighted global graph.</li>
          <li>Baseline: Micro Blossom <code>SolverEmbeddedComb</code> on the entire global graph. Comparison: the existing open-top then closed-bottom CPU window decoder.</li>
          <li>Per-shot monotonic timing covers decoder calls: solver construction, solve and subgraph extraction, plus local routing and seam stitching in the window path. It excludes Stim, graph conversion, external parity checks and file I/O. Order alternates.</li>
          <li>Every correction must clear the global residual. Logical accuracy is the fraction of truth-labeled input shots with an exact observable-flip match; nonempty/all views change its population, never its timed sample count.</li></ol>
        <p>No ground truth means N/A, not 0% accuracy. Small supplied-shot sets and shared-host jitter limit conclusions. No CPU concurrency, FPGA scheduler, FPGA measurement, or hardware extrapolation.</p>
      </section>
      <section className="bench-panel bench-command"><div className="bench-panel-header"><strong>Reproduce in the terminal</strong><span>RELEASE BUILD RECOMMENDED</span></div>
        <pre><code>{command}</code></pre>
        <p>Run at the repository root. The tiny input is derived from captured actual Stim shots, not from decoder predictions. From the repository root, set <code>MICROBLOSSOM_TOOLCHAIN_BINARY="$PWD/target/release/microblossom-toolchain"</code> before launching Vite to run the comparison in the UI.</p>
      </section>
    </div>
  </section>;
}
