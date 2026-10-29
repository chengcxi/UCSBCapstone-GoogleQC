import { execFile } from 'node:child_process';
import { constants, existsSync } from 'node:fs';
import { access, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';
import react from '@vitejs/plugin-react';
import { defineConfig, type Plugin } from 'vite';
import { shotsFromCorrectionCapture } from './src/benchmarkShots';
import { parseGenerationConfig } from './src/generationConfig';

const exec = promisify(execFile);
const repo = fileURLToPath(new URL('..', import.meta.url));
const release = path.join(repo, 'target/release/microblossom-toolchain');
const binary = process.env.MICROBLOSSOM_TOOLCHAIN_BINARY || (existsSync(release) ? release : path.join(repo, 'target/debug/microblossom-toolchain'));
const python = process.env.MICROBLOSSOM_PYTHON || (existsSync(path.join(repo, '.venv/bin/python')) ? path.join(repo, '.venv/bin/python') : 'python3');
const runner = process.env.MICROBLOSSOM_ACCURACY_BINARY || path.join(repo, 'target/release/examples/window_accuracy');

function send(res: import('node:http').ServerResponse, status: number, data: unknown) {
  res.writeHead(status, { 'Content-Type': 'application/json; charset=utf-8', 'Cache-Control': 'no-store' });
  res.end(JSON.stringify(data));
}

async function availability() {
  let binaryPresent = false;
  let stimPresent = false;
  let generatorDependencies = false;
  let runnerPresent = false;
  try {
    await access(binary, constants.X_OK);
    binaryPresent = true;
  } catch { /* The saved CLI captures still work without a local Rust build. */ }
  try {
    await exec(python, ['-c', 'import stim'], { timeout: 4000 });
    stimPresent = true;
  } catch { /* Stim is only needed to run new samples. */ }
  try { await access(runner, constants.X_OK); runnerPresent = true; } catch { /* Display setup instructions. */ }
  try { await exec(python, ['-c', 'import stim, numpy, fusion_blossom'], { timeout: 4000 }); generatorDependencies = true; } catch { /* Optional graph generator dependencies. */ }
  return {
    generationAvailable: binaryPresent && runnerPresent && generatorDependencies,
    generationReason: !binaryPresent || !runnerPresent
      ? 'Run cargo build --release --locked and cargo build --release --locked --example window_accuracy.'
      : !generatorDependencies ? 'Install requirements-dev.txt in .venv, or set MICROBLOSSOM_PYTHON to Python with Stim, NumPy and Fusion Blossom.' : null,
    liveAvailable: binaryPresent && stimPresent,
    benchmarkAvailable: binaryPresent,
    benchmarkReason: binaryPresent ? null : 'Build the Rust CLI with cargo build --release --locked, then set MICROBLOSSOM_TOOLCHAIN_BINARY to the absolute path of target/release/microblossom-toolchain.',
    binaryPresent,
    stimPresent,
    reason: !binaryPresent
      ? 'Build the Rust CLI with cargo build --locked to run a fresh shot.'
      : !stimPresent
        ? 'Install Stim with python -m pip install -r requirements.txt to run a fresh shot.'
        : null,
  };
}

async function readBody(req: import('node:http').IncomingMessage) {
  let text = '';
  for await (const part of req) {
    text += part.toString();
    if (text.length > 2048) throw new Error('Request too large');
  }
  return JSON.parse(text) as Record<string, unknown>;
}

/** Dev-only, fixed-fixture bridge. Never accepts a path or shell command from the browser. */
function toolchainApi(): Plugin {
  let generating = false;
  return {
    name: 'local-toolchain-api',
    configureServer(server) {
      server.middlewares.use(async (req, res, next) => {
        if (req.url?.split('?')[0] === '/api/status' && req.method === 'GET') {
          send(res, 200, await availability());
          return;
        }
        if (req.url?.split('?')[0] === '/api/generate') {
          if (req.method !== 'POST') return send(res, 405, { error: 'POST required' });
          if (req.headers['sec-fetch-site'] === 'cross-site'
              || (req.headers.origin && new URL(req.headers.origin).host !== req.headers.host)) {
            return send(res, 403, { error: 'Generation requests must originate from this workbench' });
          }
          if (generating) return send(res, 409, { error: 'A graph generation job is already running' });
          let config;
          try { config = parseGenerationConfig(await readBody(req)); }
          catch (error) { return send(res, 400, { error: (error as Error).message }); }
          generating = true;
          let directory: string | undefined;
          try {
            const state = await availability();
            if (!state.generationAvailable) return send(res, 503, { error: state.generationReason });
            directory = await mkdtemp(path.join(tmpdir(), 'microblossom-generated-'));
            const configPath = path.join(directory, 'config.json');
            await writeFile(configPath, JSON.stringify(config));
            await exec(python, [path.join(repo, 'tools/generate_workbench.py'), '--config', configPath,
              '--output', directory, '--binary', binary, '--runner', runner],
            { cwd: repo, timeout: 420_000, maxBuffer: 2 * 1024 * 1024 });
            send(res, 200, JSON.parse(await readFile(path.join(directory, 'comparison.json'), 'utf8')));
          } catch (error) {
            const failure = error as Error & { stderr?: string };
            send(res, 422, { error: (failure.stderr || failure.message).trim().slice(-2000) });
          } finally {
            generating = false;
            if (directory) await rm(directory, { recursive: true, force: true });
          }
          return;
        }
        if (req.url?.split('?')[0] === '/api/benchmark') {
          if (req.method !== 'POST') return send(res, 405, { error: 'POST required' });
          const state = await availability();
          if (!state.benchmarkAvailable) return send(res, 503, { error: state.benchmarkReason });
          let directory: string | undefined;
          try {
            const { scenario, warmup = 2, repeats = 20 } = await readBody(req);
            if (scenario !== 'tiny' && scenario !== 'seam') throw new Error('Unknown fixture');
            if (!Number.isInteger(warmup) || Number(warmup) < 0 || Number(warmup) > 10) {
              throw new Error('Warmup passes must be an integer from 0 to 10');
            }
            if (!Number.isInteger(repeats) || Number(repeats) < 1 || Number(repeats) > 100) {
              throw new Error('Measured passes must be an integer from 1 to 100');
            }
            // Reuse the original *actual* CLI shot inputs; predictions are never
            // substituted for Stim/supplied observables and no paths come from the browser.
            const capture = JSON.parse(await readFile(path.join(repo, 'demo/public/fixtures', `${scenario}.corrections.json`), 'utf8'));
            const shots = shotsFromCorrectionCapture(capture);
            if (shots.shots.length * Number(repeats) > 5_000) throw new Error('At most 5000 timed shots per method');
            directory = await mkdtemp(path.join(tmpdir(), 'microblossom-benchmark-'));
            const inputs = path.join(directory, 'shots.json');
            const report = path.join(directory, 'report.json');
            await writeFile(inputs, JSON.stringify(shots));
            const ir = path.join(repo, 'demo/public/fixtures', `${scenario}.ir.json`);
            await exec(binary, ['benchmark', ir, '--shots', inputs, '--warmup', String(warmup),
              '--repeats', String(repeats), '--output', report],
            { cwd: repo, timeout: 120_000, maxBuffer: 1024 * 1024 });
            send(res, 200, JSON.parse(await readFile(report, 'utf8')));
          } catch (error) {
            const failure = error as Error & { stderr?: string };
            send(res, /Unknown fixture|Warmup passes|Measured passes|timed shots|Request too large/.test(failure.message) ? 400 : 422,
              { error: (failure.stderr || failure.message).trim().slice(0, 1200) });
          } finally {
            if (directory) await rm(directory, { recursive: true, force: true });
          }
          return;
        }
        if (req.url?.split('?')[0] !== '/api/run') return next();
        if (req.method !== 'POST') return send(res, 405, { error: 'POST required' });
        const state = await availability();
        if (!state.liveAvailable) return send(res, 503, { error: state.reason });

        let directory: string | undefined;
        try {
          const { scenario, seed = 7, count = 24 } = await readBody(req);
          if (scenario !== 'tiny' && scenario !== 'seam') throw new Error('Unknown fixture');
          if (!Number.isInteger(seed) || Number(seed) < 0 || Number(seed) > 4294967295) {
            throw new Error('Seed must be a nonnegative 32-bit integer');
          }
          if (!Number.isInteger(count) || Number(count) < 1 || Number(count) > 64) {
            throw new Error('Shot count must be between 1 and 64');
          }
          directory = await mkdtemp(path.join(tmpdir(), 'microblossom-demo-'));
          const prefix = path.join(directory, scenario);
          const source = scenario === 'tiny' ? 'examples/tiny.stim' : 'examples/seam.dem';
          const plan = scenario === 'tiny' ? 'examples/tiny-windows.json' : 'examples/windows.json';
          const args = ['compile', path.join(repo, source)];
          if (scenario === 'seam') args.push('--kind', 'dem');
          args.push('--windows', path.join(repo, plan));
          if (scenario === 'tiny') {
            args.push('--sample-shots', String(count), '--seed', String(seed));
          } else {
            args.push('--shots', path.join(repo, 'examples/seam-shots.json'));
          }
          args.push('--python', python, '--output', prefix);
          const started = performance.now();
          await exec(binary, args, { cwd: repo, timeout: 90_000, maxBuffer: 2 * 1024 * 1024 });
          const [ir, graph, windows, corrections, syndromes] = await Promise.all(
            ['ir.json', 'graph.json', 'windows.json', 'corrections.json', 'syndromes'].map(suffix =>
              readFile(`${prefix}.${suffix}`, 'utf8')),
          );
          send(res, 200, {
            mode: 'live', durationMs: Math.round(performance.now() - started),
            artifacts: {
              ir: JSON.parse(ir), graph: JSON.parse(graph), windows: JSON.parse(windows),
              corrections: JSON.parse(corrections), syndromes,
            },
          });
        } catch (error) {
          const failure = error as Error & { stderr?: string };
          send(res, /Unknown fixture|Seed must|Shot count|Request too large/.test(failure.message) ? 400 : 422,
            { error: (failure.stderr || failure.message).trim().slice(0, 1200) });
        } finally {
          if (directory) await rm(directory, { recursive: true, force: true });
        }
      });
    },
  };
}

export default defineConfig({
  plugins: [react(), toolchainApi()],
  base: './',
  server: {
    host: '0.0.0.0', port: 4173, strictPort: true, allowedHosts: true,
    fs: { allow: [repo] },
  },
});
