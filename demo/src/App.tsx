import { useEffect, useMemo, useState } from 'react';
import {
  Activity, ArrowLeft, ArrowRight, ArrowUpRight, Braces, Check, CheckCircle2,
  ChevronLeft, ChevronRight, CircleHelp, Clipboard, Code2, Download, FileCode2,
  FileJson2, FileText, GitBranch, Layers2, LoaderCircle, Pause, Play, Radio,
  RotateCcw, ScanLine, ShieldCheck, Terminal, X,
} from 'lucide-react';
import { GraphCanvas } from './GraphCanvas';
import { BenchmarkWorkbench } from './BenchmarkWorkbench';
import { loadCapture, scenarios, STAGES, type ScenarioId } from './data';
import { detectorList, edgeList, observableList, traceShot, type Artifacts, type Bundle, type Shot, type ShotTrace } from './model';

const artifactNames = ['ir', 'graph', 'windows', 'corrections', 'syndromes'] as const;
type ArtifactName = typeof artifactNames[number];
type CodeTab = 'source' | 'shots' | 'ir' | 'plan' | 'command';
type Availability = { liveAvailable: boolean; benchmarkAvailable: boolean; benchmarkReason: string | null; binaryPresent: boolean; stimPresent: boolean; reason: string | null };
type WorkspaceView = 'decoder' | 'benchmark';

const artifactMeta: Record<ArtifactName, { title: string; subtitle: string; icon: typeof FileJson2; stage: number }> = {
  ir: { title: 'IR / DEM', subtitle: 'Normalized errors + annotations', icon: Braces, stage: 1 },
  graph: { title: 'Graph', subtitle: 'Weighted global edges', icon: GitBranch, stage: 2 },
  windows: { title: 'Windows', subtitle: 'Two cut local graphs', icon: Layers2, stage: 2 },
  corrections: { title: 'Corrections', subtitle: 'Verified edges + parity', icon: CheckCircle2, stage: 5 },
  syndromes: { title: 'Parser input', subtitle: 'Syndrome Pattern v1.0', icon: FileText, stage: 2 },
};

function BrandMark({ small = false }: { small?: boolean }) {
  return (
    <span className={`brand-mark ${small ? 'brand-mark-small' : ''}`} aria-hidden="true">
      <svg viewBox="0 0 48 48" fill="none">
        <path d="M24 3C31 10 31 17 24 24C17 17 17 10 24 3Z" />
        <path d="M45 24C38 31 31 31 24 24C31 17 38 17 45 24Z" />
        <path d="M24 45C17 38 17 31 24 24C31 31 31 38 24 45Z" />
        <path d="M3 24C10 17 17 17 24 24C17 31 10 31 3 24Z" />
        <circle cx="24" cy="24" r="3.3" />
      </svg>
    </span>
  );
}

function fmtShot(index: number) { return `#${String(index + 1).padStart(2, '0')}`; }

function stageExplanation(stage: number, id: ScenarioId, shot: Shot, trace: ShotTrace, artifacts: Artifacts) {
  const top = artifacts.windows.windows.find(window => window.layer === 0);
  const bottom = artifacts.windows.windows.find(window => window.layer === 1);
  const info = [
    id === 'tiny'
      ? 'A noisy repeated measurement is written in Stim. X_ERROR instructions specify the noise; DETECTOR and OBSERVABLE_INCLUDE define what gets measured. Nothing has been decoded yet.'
      : 'This example starts at Stim’s detector-error-model format. Three time-ordered detectors and three error mechanisms make the seam visible without hiding it in a large circuit.',
    `Stim extracts graphlike error mechanisms and the bridge normalizes them into a versioned IR. The plan annotates ${artifacts.graph.num_detectors} detector IDs; annotations alone are not a correction.`,
    `Rust builds one graph of ${artifacts.graph.num_detectors} detectors and ${artifacts.graph.edge_metadata.length} weighted edges. The bands annotate where ${top?.window_id ?? 'top'} (open) and ${bottom?.window_id ?? 'bottom'} (closed) cut that same topology; shared D IDs are not duplicated.`,
    `The open top window receives the original syndrome. Its inner CPU solver may use the buffer, but only edges inside its commit interval survive: ${edgeList(trace.topEdges)}. Tentative, discarded edges are not exported and are not drawn here.`,
    `After the top commit, S ⊕ ∂C₀ = ${detectorList(trace.topResidual)}. The closed bottom window decodes that residual with no temporal escape and commits ${edgeList(trace.bottomEdges)} in full.`,
    `The CLI independently checks all committed edges against the global graph. The final residual is ${detectorList(trace.finalResidual)} and the predicted logical flips are ${observableList(shot.predicted_observables)}. Only then does it publish the correction shot.`,
  ];
  return info[stage];
}

function ArtifactDialog({ name, artifacts, scenario, onClose, onCopy, copied }: {
  name: ArtifactName; artifacts: Artifacts; scenario: ScenarioId; onClose: () => void; onCopy: (text: string) => void; copied: boolean;
}) {
  const content = name === 'syndromes' ? artifacts.syndromes : JSON.stringify(artifacts[name], null, 2);
  const fileName = `${scenario}.${name === 'syndromes' ? 'syndromes' : `${name}.json`}`;
  function download() {
    const blob = new Blob([content + (content.endsWith('\n') ? '' : '\n')], { type: name === 'syndromes' ? 'text/plain' : 'application/json' });
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement('a');
    anchor.href = url;
    anchor.download = fileName;
    anchor.click();
    setTimeout(() => URL.revokeObjectURL(url), 0);
  }
  return (
    <div className="modal-backdrop" onMouseDown={onClose}>
      <div className="artifact-modal" role="dialog" aria-modal="true" aria-label={`${fileName} artifact`} onMouseDown={event => event.stopPropagation()}>
        <div className="modal-header">
          <div><div className="micro-label">REAL CLI OUTPUT / {scenario.toUpperCase()}</div><h2>{fileName}</h2></div>
          <button className="icon-button" onClick={onClose} aria-label="Close artifact"><X size={19} /></button>
        </div>
        <div className="modal-tools"><span>Read-only · exported by microblossom-toolchain</span><div><button onClick={() => onCopy(content)}><Clipboard size={14} /> {copied ? 'Copied' : 'Copy'}</button><button onClick={download}><Download size={14} /> Download</button></div></div>
        <pre className="artifact-content"><code>{content}</code></pre>
      </div>
    </div>
  );
}

function App() {
  const [workspaceView, setWorkspaceView] = useState<WorkspaceView>('decoder');
  const [scenarioId, setScenarioId] = useState<ScenarioId>('seam');
  const [stage, setStage] = useState(3);
  const [shotIndex, setShotIndex] = useState(0);
  const [bundles, setBundles] = useState<Partial<Record<ScenarioId, Bundle>>>({});
  const [loadError, setLoadError] = useState<string | null>(null);
  const [isPlaying, setIsPlaying] = useState(false);
  const [codeTab, setCodeTab] = useState<CodeTab>('source');
  const [artifactOpen, setArtifactOpen] = useState<ArtifactName | null>(null);
  const [setupOpen, setSetupOpen] = useState(false);
  const [copied, setCopied] = useState(false);
  const [availability, setAvailability] = useState<Availability | null>(null);
  const [running, setRunning] = useState(false);
  const [runError, setRunError] = useState<string | null>(null);
  const [seed, setSeed] = useState(7);

  const bundle = bundles[scenarioId];
  const artifacts = bundle?.artifacts;
  const shots = artifacts?.corrections.shots ?? [];
  const shot = shots[Math.min(shotIndex, Math.max(0, shots.length - 1))];
  const traced = useMemo(() => {
    if (!artifacts || !shot) return { trace: null, error: null };
    try { return { trace: traceShot(artifacts, shot), error: null }; }
    catch (error) { return { trace: null, error: (error as Error).message }; }
  }, [artifacts, shot]);
  const trace = traced.trace;
  const scenario = scenarios[scenarioId];
  const irProvenance = artifacts?.ir.provenance as { input_sha256?: unknown } | undefined;
  const sourceHash = typeof irProvenance?.input_sha256 === 'string' ? irProvenance.input_sha256 : undefined;

  useEffect(() => {
    if (bundles[scenarioId]) return;
    let active = true;
    setLoadError(null);
    loadCapture(scenarioId).then(loaded => {
      if (!active) return;
      setBundles(current => ({ ...current, [scenarioId]: loaded }));
      if (scenarioId === 'tiny') {
        const examples = loaded.artifacts.corrections.shots;
        const bothLayers = examples.findIndex(item => item.committed_edges.length >= 2);
        const anyDefect = examples.findIndex(item => item.defects.length > 0);
        setShotIndex(Math.max(0, bothLayers >= 0 ? bothLayers : anyDefect));
      }
    }).catch(error => { if (active) setLoadError((error as Error).message); });
    return () => { active = false; };
  }, [scenarioId, bundles]);

  useEffect(() => {
    fetch('/api/status').then(response => response.ok ? response.json() : null)
      .then(data => setAvailability(data as Availability | null)).catch(() => setAvailability(null));
  }, []);

  useEffect(() => {
    if (!isPlaying) return;
    const timer = window.setInterval(() => {
      setStage(current => {
        if (current >= STAGES.length - 1) { setIsPlaying(false); return current; }
        return current + 1;
      });
    }, 2250);
    return () => window.clearInterval(timer);
  }, [isPlaying]);

  useEffect(() => {
    function keydown(event: KeyboardEvent) {
      if (artifactOpen || setupOpen) {
        if (event.key === 'Escape') { setArtifactOpen(null); setSetupOpen(false); }
        return;
      }
      if (workspaceView !== 'decoder' || ['INPUT', 'TEXTAREA', 'SELECT'].includes((event.target as HTMLElement)?.tagName)) return;
      if (event.key === 'ArrowRight') { setIsPlaying(false); setStage(value => Math.min(5, value + 1)); }
      if (event.key === 'ArrowLeft') { setIsPlaying(false); setStage(value => Math.max(0, value - 1)); }
    }
    window.addEventListener('keydown', keydown);
    return () => window.removeEventListener('keydown', keydown);
  }, [artifactOpen, setupOpen, workspaceView]);

  useEffect(() => {
    if (stage === 0) setCodeTab('source');
    if (stage === 1) setCodeTab('ir');
    if (stage === 2) setCodeTab('plan');
    if (stage === 5) setCodeTab('command');
  }, [stage]);

  function chooseScenario(id: ScenarioId) {
    if (id === scenarioId) return;
    setIsPlaying(false); setScenarioId(id); setShotIndex(0); setStage(3); setCodeTab('source'); setRunError(null);
  }
  function chooseStage(index: number) { setIsPlaying(false); setWorkspaceView('decoder'); setStage(index); }
  function play() {
    if (isPlaying) return setIsPlaying(false);
    if (stage >= 5) setStage(0);
    setIsPlaying(true);
  }
  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1900);
    } catch { setRunError('Clipboard access is unavailable in this browser.'); }
  }
  async function runLive() {
    if (!availability?.liveAvailable) { setSetupOpen(true); return; }
    setRunning(true); setRunError(null); setIsPlaying(false);
    try {
      const response = await fetch('/api/run', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ scenario: scenarioId, seed, count: 24 }) });
      const result = await response.json();
      if (!response.ok) throw new Error(result.error || 'The Rust CLI could not complete this run.');
      const next = result as Bundle;
      // Reject an inconsistent response rather than animating a fake correction.
      next.artifacts.corrections.shots.forEach(item => traceShot(next.artifacts, item));
      setBundles(current => ({ ...current, [scenarioId]: next }));
      const examples = next.artifacts.corrections.shots;
      const bothLayers = examples.findIndex(item => item.committed_edges.length >= 2);
      const anyDefect = examples.findIndex(item => item.defects.length > 0);
      setShotIndex(Math.max(0, bothLayers >= 0 ? bothLayers : anyDefect)); setStage(3);
    } catch (error) { setRunError((error as Error).message); }
    finally { setRunning(false); }
  }
  async function restoreCapture() {
    try {
      const saved = await loadCapture(scenarioId);
      setBundles(current => ({ ...current, [scenarioId]: saved }));
      setShotIndex(0); setRunError(null);
    } catch (error) { setRunError((error as Error).message); }
  }

  const code = codeTab === 'source' ? scenario.sourceText : codeTab === 'shots' ? scenario.shotsText ?? '' : codeTab === 'ir' ? JSON.stringify(artifacts?.ir ?? {}, null, 2) : codeTab === 'plan' ? scenario.planText : scenario.command;
  const codeFile = codeTab === 'source' ? scenario.sourceFile : codeTab === 'shots' ? 'examples/seam-shots.json' : codeTab === 'ir' ? `${scenarioId}.ir.json` : codeTab === 'plan' ? scenario.planFile : 'terminal';
  const codeTabs: { tab: CodeTab; label: string }[] = [
    { tab: 'source', label: scenarioId === 'tiny' ? 'Stim circuit' : 'Stim DEM' },
    ...(scenarioId === 'seam' ? [{ tab: 'shots' as const, label: 'Input shots' }] : []),
    { tab: 'ir', label: 'Normalized IR' },
    { tab: 'plan', label: 'Window plan' },
    { tab: 'command', label: 'CLI command' },
  ];
  const selectedShot = shots.length ? Math.min(shotIndex, shots.length - 1) : 0;
  const visibleShots = shots.length <= 6 ? shots.map((_, index) => index) : Array.from({ length: 5 }, (_, offset) => {
    const first = Math.max(0, Math.min(shots.length - 5, selectedShot - 2));
    return first + offset;
  });

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand"><BrandMark /><div><strong>microblossom</strong><span>DECODER LAB</span></div></div>
        <div className="sidebar-section-label">A GUIDED RUN <span>01 — 06</span></div>
        <nav aria-label="Pipeline stages" className="stage-nav">
          {STAGES.map((item, index) => {
            const selected = workspaceView === 'decoder' && stage === index;
            return <button key={item.short} className={`stage-link ${selected ? 'selected' : ''} ${workspaceView === 'decoder' && index < stage ? 'completed' : ''}`} onClick={() => chooseStage(index)} aria-current={selected ? 'step' : undefined}>
              <span className="stage-node">{index < stage ? <Check size={12} strokeWidth={2.5} /> : String(index + 1).padStart(2, '0')}</span>
              <span className="stage-text"><strong>{item.short}</strong><small>{item.note}</small></span>
              {selected && <span className="stage-indicator" />}
            </button>;
          })}
        </nav>
        <div className="sidebar-tools"><div className="sidebar-section-label">ANALYSIS</div>
          <button className={`stage-link bench-nav ${workspaceView === 'benchmark' ? 'selected' : ''}`}
            onClick={() => { setIsPlaying(false); setWorkspaceView('benchmark'); }} aria-current={workspaceView === 'benchmark' ? 'page' : undefined}>
            <span className="stage-node">∿</span><span className="stage-text"><strong>CPU benchmark</strong><small>Global vs. windowed</small></span>
          </button><div className="sidebar-fpga">FPGA parallel windows <strong>NOT MEASURED</strong></div>
        </div>
        <div className="sidebar-divider" />
        <div className="sidebar-observation"><span className="tiny-pulse" /> SOFTWARE PATH <span className="sidebar-observation-right">v0.1</span></div>
        <p className="sidebar-fineprint">Two decoding layers. One global correction. CPU windows execute in sequence today; hardware parallelism is future work.</p>
        <div className="sidebar-footer"><span>MB.</span><span>BUILD THE UNDERSTANDING</span></div>
      </aside>

      <div className="workspace">
        <header className="topbar">
          <div className="breadcrumbs"><strong>MICRO BLOSSOM</strong><span className="crumb-slash">/</span><span className="breadcrumb-muted">CPU TOOLCHAIN</span></div>
          <div className="workspace-tabs" role="tablist" aria-label="Workbench view">
            <button role="tab" aria-selected={workspaceView === 'decoder'} className={workspaceView === 'decoder' ? 'active' : ''} onClick={() => setWorkspaceView('decoder')}>Decoder</button>
            <button role="tab" aria-selected={workspaceView === 'benchmark'} className={workspaceView === 'benchmark' ? 'active' : ''} onClick={() => { setIsPlaying(false); setWorkspaceView('benchmark'); }}>Benchmark</button>
          </div>
          <div className="topbar-right"><span className="topbar-status"><Radio size={13} /> {workspaceView === 'benchmark' ? (availability?.benchmarkAvailable ? 'LOCAL CPU READY' : 'IMPORT OR BUILD') : (availability?.liveAvailable ? 'LOCAL CLI READY' : 'CAPTURED CLI OUTPUT')}</span><button className="topbar-help" onClick={() => setSetupOpen(true)}><CircleHelp size={15} /> Help</button></div>
        </header>

        <main className="main-content">
          {workspaceView === 'decoder' ? <>
          <section className="hero">
            <div className="hero-copy"><div className="eyebrow">WORKSPACE / DECODER</div>
              <h1>Decoder workflow</h1>
              <p>Stim input → normalized IR → global detector graph → two sequential CPU windows → verified correction.</p>
            </div>
          </section>

          <section className="control-deck" aria-label="Demo controls">
            <div className="scenario-control"><span className="control-label">CHOOSE A PATH</span><div className="scenario-buttons">
              <button className={`scenario-button ${scenarioId === 'seam' ? 'active' : ''}`} onClick={() => chooseScenario('seam')} aria-pressed={scenarioId === 'seam'}><GitBranch size={17} /><span><strong>Seam study</strong><small>Supplied DEM shots</small></span></button>
              <button className={`scenario-button ${scenarioId === 'tiny' ? 'active' : ''}`} onClick={() => chooseScenario('tiny')} aria-pressed={scenarioId === 'tiny'}><FileCode2 size={17} /><span><strong>Stim circuit</strong><small>24 seeded samples</small></span></button>
            </div></div>
            <div className="deck-separator" />
            <div className="shot-control"><div className="shot-header"><span className="control-label">PICK A SYNDROME</span><span className="shot-count">{shots.length ? `${shots.length} real shots` : 'Loading…'}</span></div><div className="shot-row">
              <button className="shot-arrow" onClick={() => setShotIndex(Math.max(0, selectedShot - 1))} disabled={selectedShot === 0} aria-label="Previous shot"><ChevronLeft size={17} /></button>
              {visibleShots.map(index => <button key={index} className={`shot-chip ${selectedShot === index ? 'active' : ''}`} onClick={() => setShotIndex(index)} aria-label={`Shot ${index + 1}: ${detectorList(shots[index].defects)}`} aria-pressed={selectedShot === index}>{fmtShot(index)}</button>)}
              <button className="shot-arrow" onClick={() => setShotIndex(Math.min(shots.length - 1, selectedShot + 1))} disabled={selectedShot >= shots.length - 1} aria-label="Next shot"><ChevronRight size={17} /></button>
              {shots.length > 6 && <select className="shot-select" value={selectedShot} onChange={event => setShotIndex(Number(event.target.value))} aria-label="Choose any captured shot">{shots.map((item, index) => <option key={index} value={index}>Shot {String(index + 1).padStart(2, '0')} · {detectorList(item.defects)}</option>)}</select>}
            </div></div>
            <div className="deck-separator action-separator" />
            <div className="deck-action"><span className="control-label">WALK THROUGH</span><button className="play-button" onClick={play}>{isPlaying ? <Pause size={15} fill="currentColor" /> : <Play size={15} fill="currentColor" />} {isPlaying ? 'Pause tour' : 'Play sequence'}</button></div>
          </section>

          {(loadError || runError || traced.error) && <div className="error-banner" role="alert"><CircleHelp size={17} /><span>{loadError || runError || traced.error}</span>{loadError && <button onClick={() => { setLoadError(null); loadCapture(scenarioId).then(saved => setBundles(current => ({ ...current, [scenarioId]: saved }))).catch(error => setLoadError((error as Error).message)); }}>Retry</button>}</div>}

          {artifacts && shot && trace ? <>
            <div className="section-heading"><div><span className="micro-label">THE DECODING WORKBENCH</span><h2>Follow the evidence.</h2></div><div className="section-meta"><span className="source-dot" /> {bundle?.mode === 'live' ? `LIVE RUST CLI · ${bundle.durationMs} MS` : 'VERIFIED CLI CAPTURE · REPLAY'}</div></div>
            <div className="analysis-grid">
              <section className="visual-panel" aria-label="Global detector graph with two window annotations">
                <div className="panel-head"><div><ScanLine size={17} /><span>Global detector graph</span><span className="panel-head-minor">/ WINDOW OVERLAYS</span></div><div className="panel-counter">{String(stage + 1).padStart(2, '0')} <span>/ 06</span></div></div>
                <GraphCanvas artifacts={artifacts} shot={shot} trace={trace} stage={stage} />
                <div className="graph-footer"><div><span className="graph-footer-label">INPUT SYNDROME</span><strong>{detectorList(shot.defects)}</strong></div><ArrowRight size={16} className="graph-footer-arrow" /><div><span className="graph-footer-label">SEAM RESIDUAL</span><strong className={stage >= 3 ? 'value-warm' : ''}>{stage >= 3 ? detectorList(trace.topResidual) : '—'}</strong></div><ArrowRight size={16} className="graph-footer-arrow" /><div><span className="graph-footer-label">FINAL RESIDUAL</span><strong className={stage >= 5 ? 'value-good' : ''}>{stage >= 5 ? detectorList(trace.finalResidual) : '—'}</strong></div></div>
              </section>
              <section className="inspector" aria-label="Current stage details">
                <div className="inspector-top"><span className="inspector-kicker">{STAGES[stage].eyebrow}</span><span className={`layer-badge ${stage === 3 ? 'badge-top' : stage === 4 ? 'badge-bottom' : 'badge-neutral'}`}>{stage === 3 ? 'OPEN WINDOW' : stage === 4 ? 'CLOSED WINDOW' : 'SOFTWARE TRACE'}</span></div>
                <h2>{STAGES[stage].title}</h2>
                <p className="inspector-description">{stageExplanation(stage, scenarioId, shot, trace, artifacts)}</p>
                <div className="trace-table">
                  <div className="trace-row"><span><i className="trace-icon input-icon"><Activity size={14} /></i> Input syndrome</span><strong>{detectorList(shot.defects)}</strong></div>
                  <div className={`trace-row ${stage === 3 ? 'trace-emphasis' : ''}`}><span><i className="trace-icon top-icon"><GitBranch size={14} /></i> Top committed</span><strong className="teal-value">{stage >= 3 ? edgeList(trace.topEdges) : '—'}</strong></div>
                  <div className={`trace-row ${stage === 4 ? 'trace-emphasis' : ''}`}><span><i className="trace-icon seam-icon"><ArrowRight size={14} /></i> Seam residual</span><strong>{stage >= 3 ? detectorList(trace.topResidual) : '—'}</strong></div>
                  <div className={`trace-row ${stage === 4 ? 'trace-emphasis' : ''}`}><span><i className="trace-icon bottom-icon"><Layers2 size={14} /></i> Bottom committed</span><strong className="amber-value">{stage >= 4 ? edgeList(trace.bottomEdges) : '—'}</strong></div>
                </div>
                <div className={`result-band ${stage >= 5 ? 'result-revealed' : ''}`}><div><ShieldCheck size={18} /><span>GLOBAL CHECK</span></div><strong>{stage >= 5 ? `S ⊕ ∂C = ${detectorList(trace.finalResidual)}` : 'Awaiting final commit'}</strong></div>
                <div className="inspector-bottom"><button className="step-button prev" onClick={() => chooseStage(stage - 1)} disabled={stage === 0}><ArrowLeft size={16} /> Previous</button><span className="inspector-progress">{stage + 1} of 6</span><button className="step-button next" onClick={() => chooseStage(stage + 1)} disabled={stage === 5}>Next step <ArrowRight size={16} /></button></div>
              </section>
            </div>

            <div className="lower-grid">
              <section className="code-panel"><div className="lower-head"><div><Code2 size={17} /><h3>At the source</h3></div><span>REPRODUCIBLE INPUT</span></div><div className="code-tabs" role="tablist" aria-label="Source files">
                {codeTabs.map(({ tab, label }) => <button key={tab} role="tab" aria-selected={codeTab === tab} className={codeTab === tab ? 'active' : ''} onClick={() => setCodeTab(tab)}>{label}</button>)}
              </div><div className="code-filebar"><span><span className="file-indicator" /> {codeFile}</span><button onClick={() => copy(code)} aria-label={`Copy ${codeFile}`}><Clipboard size={14} /> {copied ? 'Copied' : 'Copy'}</button></div><pre className="code-content"><code>{code.split('\n').map((line, index) => <span className="code-line" key={index}><span className="line-number">{String(index + 1).padStart(2, '0')}</span><span>{line || ' '}</span></span>)}</code></pre></section>
              <section className="artifact-panel"><div className="lower-head"><div><FileJson2 size={17} /><h3>Artifact trail</h3></div><span>RUST CLI EXPORT</span></div><p>Each file below is loaded from the CLI run behind this walkthrough. Open it to inspect the actual data.</p><div className="artifact-list">
                {artifactNames.map(name => { const meta = artifactMeta[name]; const Icon = meta.icon; return <button key={name} className={`artifact-item ${stage === meta.stage ? 'artifact-current' : ''}`} onClick={() => { setCopied(false); setArtifactOpen(name); }}><span className="artifact-icon"><Icon size={18} /></span><span className="artifact-words"><strong>{scenarioId}.{name === 'syndromes' ? 'syndromes' : `${name}.json`}</strong><small>{meta.subtitle}</small></span><ArrowUpRight size={17} /></button>; })}
              </div><div className="artifact-bottom"><ShieldCheck size={15} /> Indexed by the same global detector and edge IDs.</div></section>
            </div>

            <section className="run-strip"><div className="run-strip-icon"><Terminal size={21} /></div><div className="run-strip-text"><span className="micro-label">GO BEYOND THE REPLAY</span><h3>Run the exact pipeline yourself.</h3><p>{bundle?.mode === 'live' ? 'This result came from your locally built Rust binary.' : 'These are saved outputs from a real CLI run. Build the binary to rerun this example here.'}</p></div><div className="run-strip-actions">{bundle?.mode === 'live' && <button className="restore-button" onClick={restoreCapture}><RotateCcw size={14} /> Restore capture</button>}{scenarioId === 'tiny' && availability?.liveAvailable && <label className="seed-field">SEED <input type="number" min="0" max="4294967295" value={seed} onChange={event => setSeed(Number(event.target.value))} aria-label="Stim sample seed" /></label>}<button className="copy-command" onClick={() => copy(scenario.command)}><Clipboard size={15} /> Copy CLI command</button><button className="run-live" onClick={runLive} disabled={running}>{running ? <LoaderCircle className="spin" size={16} /> : <ArrowUpRight size={16} />} {running ? 'Decoding…' : 'Run CLI locally'}</button></div></section>
            <div className="honesty-note"><span>IMPORTANT DISTINCTION</span> The diagram draws each vertex and edge once from the exported global graph. Bands mark the local window membership; a dashed T callout marks only a local cut, never a global vertex. Committed edges are reconstructed from Rust’s ownership rule. Tentative solver edges and concurrent CPU/FPGA execution are not depicted.</div>
          </> : <section className="loading-panel">{loadError ? <><CircleHelp size={32} /><h2>Capture unavailable</h2><p>{loadError}</p><button onClick={() => window.location.reload()}>Reload demo</button></> : <><LoaderCircle size={30} className="spin" /><h2>Loading the CLI evidence…</h2><p>Reading the actual graph, cut windows, and correction shots.</p></>}</section>}
          </> : <BenchmarkWorkbench scenarioId={scenarioId} onScenarioChange={chooseScenario} sourceHash={sourceHash}
            graph={artifacts?.graph} binaryPresent={availability?.benchmarkAvailable ?? false} unavailableReason={availability?.benchmarkReason ?? null} />}
          <footer className="page-footer"><span><BrandMark small /> Micro Blossom · CPU Workbench</span><span>{workspaceView === 'decoder' ? 'STIM → RUST → TWO LAYERS → VERIFIED CORRECTION' : 'GLOBAL CPU  /  SEQUENTIAL WINDOW CPU  /  FPGA NOT MEASURED'}</span></footer>
        </main>
      </div>
      {artifactOpen && artifacts && <ArtifactDialog name={artifactOpen} artifacts={artifacts} scenario={scenarioId} onClose={() => setArtifactOpen(null)} onCopy={copy} copied={copied} />}
      {setupOpen && <div className="modal-backdrop" onMouseDown={() => setSetupOpen(false)}>
        <div className="setup-modal" role="dialog" aria-modal="true" aria-label={workspaceView === 'benchmark' ? 'CPU benchmark help' : 'Decoder setup help'} onMouseDown={event => event.stopPropagation()}>
          <button className="icon-button close-setup" onClick={() => setSetupOpen(false)} aria-label="Close dialog"><X size={20} /></button>
          <span className="setup-icon"><Terminal size={25} /></span>
          {workspaceView === 'benchmark' ? <>
            <span className="micro-label">CPU MEASUREMENT / NO FPGA PROJECTION</span>
            <h2>Measure both CPU paths.</h2>
            <p>The seam result is a real release-binary CI sample on a shared runner. To measure on your machine, build the Rust CLI, point Vite at its release binary, then run the comparison. Fixed captured shots are reused; Python and Stim are not needed for this benchmark.</p>
            <div className="setup-steps"><div><span>01</span><code>cargo build --release --locked</code></div><div><span>02</span><code>export MICROBLOSSOM_TOOLCHAIN_BINARY="$PWD/target/release/microblossom-toolchain"</code></div><div><span>03</span><code>npm --prefix demo run dev</code></div></div>
            <p className="setup-status">{availability?.benchmarkReason ?? (availability?.benchmarkAvailable ? 'Rust binary detected; the local CPU comparison is ready.' : 'Import a real CLI report, or build the release Rust binary to rerun.')}</p>
          </> : <>
            <span className="micro-label">REPLAY VS. LIVE DECODING</span>
            <h2>Real output, no imaginary shots.</h2>
            <p>The walkthrough reads checked-in artifacts produced by <code>microblossom-toolchain</code>. To resample and decode the selected example, launch this dev server with both the Rust binary and Python/Stim. Only fixed example inputs are accepted by the local API.</p>
            <div className="setup-steps"><div><span>01</span><code>python -m pip install -r requirements.txt</code></div><div><span>02</span><code>cargo build --locked</code></div><div><span>03</span><code>cd demo &amp;&amp; npm ci &amp;&amp; npm run dev</code></div></div>
            <p className="setup-status">{availability?.reason ?? (availability?.liveAvailable ? 'The local Rust CLI and Stim are ready.' : 'Static preview: run the Vite dev server for the local API.')}</p>
          </>}
          <button className="setup-done" onClick={() => setSetupOpen(false)}>Back to the {workspaceView === 'benchmark' ? 'comparison' : 'walkthrough'} <ArrowRight size={16} /></button>
        </div>
      </div>}
    </div>
  );
}

export default App;
